// Package cdev is the cdev Go SDK — one file, standard library only.
//
// Setup (in a module): `cdev sdk ./cdev`, then in go.mod:
//
//	require cdev v0.0.0
//	replace cdev => ./cdev
//
// and `import "cdev"`. (`cdev watch main.go` does this for you.)
//
//	total := cdev.W(price * qty)          // records "price * qty" = 42 : int, returns the value
//	cdev.W(head, "head")                   // explicit label; pointers, structs, maps, slices all work
//	func solve(n int, grid [][]int) int {
//		defer cdev.Trace(n, grid)()        // args + duration + caller, recorded on return
//		cdev.Log("n is %d", n)
//		...
//	}
//	http.ListenAndServe(":8080", cdev.Handler(mux))   // incoming requests with bodies
//	defer cdev.Flush()                     // first line of main: send anything queued before exit
//
// Outgoing requests through http.DefaultTransport are captured automatically.
// Set CDEV_OFF=1 to disable.
package cdev

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"reflect"
	"runtime"
	"strings"
	"sync"
	"time"
	"unsafe"
)

const maxBody = 16 * 1024

var (
	target  = envOr("CDEV_URL", "http://127.0.0.1:4400")
	service = envOr("CDEV_SERVICE", filepath.Base(os.Args[0]))
	off     = os.Getenv("CDEV_OFF") != ""
	cwd, _  = os.Getwd()

	mu     sync.Mutex
	queue  []map[string]any
	last   = map[string]string{}
	stacks = map[int64][]string{} // caller stack; single shared stack (key 0), best effort across goroutines
	raw    = &http.Client{Timeout: 2 * time.Second, Transport: http.DefaultTransport}
)

func envOr(k, d string) string {
	if v := os.Getenv(k); v != "" {
		return v
	}
	return d
}

func init() {
	if off {
		return
	}
	http.DefaultTransport = &transport{base: http.DefaultTransport}
	go func() {
		for {
			time.Sleep(50 * time.Millisecond)
			Flush()
		}
	}()
}

func send(ev map[string]any) {
	if off {
		return
	}
	ev["ts"] = float64(time.Now().UnixNano()) / 1e6
	ev["svc"] = service
	ev["lang"] = "go"
	ev["pid"] = os.Getpid()
	mu.Lock()
	queue = append(queue, ev)
	if len(queue) > 5000 {
		queue = queue[1:]
	}
	mu.Unlock()
}

// Flush sends queued events now. Call `defer cdev.Flush()` at the top of main.
func Flush() {
	mu.Lock()
	batch := queue
	queue = nil
	mu.Unlock()
	if len(batch) == 0 {
		return
	}
	body, err := json.Marshal(batch)
	if err != nil {
		return
	}
	req, _ := http.NewRequest("POST", target+"/ingest", bytes.NewReader(body))
	req.Header.Set("content-type", "application/json")
	req.Header.Set("x-cdev-internal", "1")
	if resp, err := raw.Do(req); err == nil {
		io.Copy(io.Discard, resp.Body)
		resp.Body.Close()
	}
}

// ---------------------------------------------------------------- values

// ser turns any value into JSON-safe data. Shared/cyclic pointers become
// {"__ref": id} pointing at the object tagged "__id".
func ser(v any, maxDepth, maxNodes int) any {
	seen := map[uintptr]map[string]any{}
	nodes, nextID := 0, 1
	var walk func(r reflect.Value, d int) any
	walk = func(r reflect.Value, d int) any {
		if !r.IsValid() {
			return nil
		}
		nodes++
		if d > maxDepth || nodes > maxNodes {
			return map[string]any{"__t": r.Type().String(), "v": "…"}
		}
		switch r.Kind() {
		case reflect.Bool:
			return r.Bool()
		case reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64:
			return r.Int()
		case reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64, reflect.Uintptr:
			return r.Uint()
		case reflect.Float32, reflect.Float64:
			return r.Float()
		case reflect.String:
			s := r.String()
			if len(s) > 2000 {
				s = s[:2000] + "…"
			}
			return s
		case reflect.Interface:
			if r.IsNil() {
				return nil
			}
			return walk(r.Elem(), d)
		case reflect.Pointer:
			if r.IsNil() {
				return nil
			}
			if out, ok := seen[r.Pointer()]; ok {
				if _, has := out["__id"]; !has {
					out["__id"] = nextID
					nextID++
				}
				return map[string]any{"__ref": out["__id"]}
			}
			if r.Elem().Kind() == reflect.Struct {
				out := map[string]any{}
				seen[r.Pointer()] = out
				fillStruct(out, r.Elem(), d, walk)
				return out
			}
			return walk(r.Elem(), d)
		case reflect.Struct:
			if r.CanInterface() {
				if t, ok := r.Interface().(time.Time); ok {
					return map[string]any{"__t": "time.Time", "v": t.Format(time.RFC3339Nano)}
				}
			}
			out := map[string]any{}
			fillStruct(out, r, d, walk)
			return out
		case reflect.Slice, reflect.Array:
			if r.Kind() == reflect.Slice && r.IsNil() {
				return []any{}
			}
			if r.Type().Elem().Kind() == reflect.Uint8 && r.Len() > 64 {
				return map[string]any{"__t": "[]byte", "length": r.Len()}
			}
			n := r.Len()
			if n > 500 {
				n = 500
			}
			out := make([]any, 0, n)
			for i := 0; i < n; i++ {
				out = append(out, walk(r.Index(i), d+1))
			}
			return out
		case reflect.Map:
			keys := r.MapKeys()
			if r.Type().Key().Kind() == reflect.String {
				out := map[string]any{}
				for i, k := range keys {
					if i >= 100 {
						break
					}
					out[k.String()] = walk(r.MapIndex(k), d+1)
				}
				return out
			}
			entries := []any{}
			for i, k := range keys {
				if i >= 100 {
					break
				}
				entries = append(entries, []any{walk(k, d+1), walk(r.MapIndex(k), d+1)})
			}
			return map[string]any{"__t": "Map", "size": r.Len(), "entries": entries}
		case reflect.Func:
			return map[string]any{"__t": "function", "name": r.Type().String()}
		default:
			return map[string]any{"__t": r.Type().String()}
		}
	}
	return walk(reflect.ValueOf(v), 0)
}

func fillStruct(out map[string]any, r reflect.Value, d int, walk func(reflect.Value, int) any) {
	t := r.Type()
	out["__class"] = t.Name()
	for i := 0; i < t.NumField() && i < 60; i++ {
		f := t.Field(i)
		name := f.Name
		switch strings.ToLower(name) {
		case "next", "prev", "left", "right", "children":
			name = strings.ToLower(name) // so the sidecar recognises lists / trees
			out[name] = walk(r.Field(i), d)
		default:
			out[name] = walk(r.Field(i), d+1)
		}
	}
}

func typeName(v any) string {
	if v == nil {
		return "nil"
	}
	return strings.ReplaceAll(reflect.TypeOf(v).String(), "main.", "")
}

func loc(skip int) (string, string, int) {
	_, file, line, ok := runtime.Caller(skip)
	if !ok {
		return "", "", 0
	}
	rel := file
	if r, err := filepath.Rel(cwd, file); err == nil && !strings.HasPrefix(r, "..") {
		rel = r
	}
	return fmt.Sprintf("%s:%d", rel, line), file, line
}

var srcCache = map[string][]string{}

// callArgs returns the source text of the arguments of `fn(` on file:line.
func callArgs(file string, line int, fn string) []string {
	lines, ok := srcCache[file]
	if !ok {
		b, err := os.ReadFile(file)
		if err != nil {
			return nil
		}
		lines = strings.Split(string(b), "\n")
		srcCache[file] = lines
	}
	if line < 1 || line > len(lines) {
		return nil
	}
	src := lines[line-1]
	i := strings.Index(src, fn+"(")
	if i < 0 {
		return nil
	}
	var out []string
	depth, cur := 0, ""
	for _, c := range src[i+len(fn)+1:] {
		switch c {
		case '(', '[', '{':
			depth++
		case ')', ']', '}':
			if depth == 0 {
				if strings.TrimSpace(cur) != "" {
					out = append(out, strings.TrimSpace(cur))
				}
				return out
			}
			depth--
		case ',':
			if depth == 0 {
				out = append(out, strings.TrimSpace(cur))
				cur = ""
				continue
			}
		}
		cur += string(c)
	}
	return out
}

// ---------------------------------------------------------------- API

// W records a value (+ type, source line) and returns it unchanged.
func W[T any](v T, label ...string) T {
	if off {
		return v
	}
	l, file, line := loc(2)
	name := ""
	if len(label) > 0 {
		name = label[0]
	} else if a := callArgs(file, line, "W"); len(a) > 0 {
		name = a[0]
	} else {
		name = l
	}
	data := ser(v, 64, 2000)
	m := memOf(v)
	b, _ := json.Marshal([]any{data, m})
	key := l + name
	mu.Lock()
	same := last[key] == string(b)
	last[key] = string(b)
	caller := currentCaller()
	mu.Unlock()
	if same {
		return v
	}
	send(map[string]any{"kind": "watch", "name": name, "v": data, "mem": m, "t": typeName(v), "loc": l, "caller": caller})
	return v
}

// memOf reports what a value points at. W receives a copy, so the variable's
// own address isn't known; pass a pointer (cdev.W(&x)) to see where x lives.
func memOf(v any) map[string]any {
	r := reflect.ValueOf(v)
	if !r.IsValid() {
		return nil
	}
	m := map[string]any{"size": r.Type().Size(), "region": "value"}
	hexp := func(p uintptr) string { return fmt.Sprintf("0x%x", p) }
	switch r.Kind() {
	case reflect.Pointer:
		if r.IsNil() {
			m["ptr"] = nil
			break
		}
		// cdev.W(&x): report x itself
		m["addr"], m["size"], m["region"] = hexp(r.Pointer()), r.Elem().Type().Size(), "heap"
		if s := r.Elem(); s.Kind() == reflect.Slice && !s.IsNil() {
			m["heap"] = map[string]any{"addr": hexp(s.Pointer()), "len": s.Len(), "cap": s.Cap(), "elem": s.Type().Elem().Size(), "region": "heap"}
		}
	case reflect.Slice:
		if !r.IsNil() {
			m["heap"] = map[string]any{"addr": hexp(r.Pointer()), "len": r.Len(), "cap": r.Cap(), "elem": r.Type().Elem().Size(), "region": "heap"}
		}
	case reflect.String:
		s := r.String()
		if len(s) > 0 {
			m["heap"] = map[string]any{"addr": hexp(uintptr(unsafe.Pointer(unsafe.StringData(s)))), "len": len(s), "cap": len(s), "elem": 1, "region": "heap"}
		}
	case reflect.Map, reflect.Chan, reflect.Func:
		m["ptr"] = hexp(r.Pointer())
	}
	return m
}

func currentCaller() any {
	s := stacks[0]
	if len(s) == 0 {
		return nil
	}
	return s[len(s)-1]
}

// Trace records the calling function's arguments and duration:
//
//	defer cdev.Trace(a, b)()
func Trace(args ...any) func() {
	if off {
		return func() {}
	}
	pc, file, line, _ := runtime.Caller(1)
	name := "?"
	if fn := runtime.FuncForPC(pc); fn != nil {
		name = fn.Name()
		if i := strings.LastIndex(name, "."); i >= 0 {
			name = name[i+1:]
		}
	}
	l, _, _ := loc(2)
	params := callArgs(file, line, "Trace")
	vals := make([]any, len(args))
	types := make([]string, len(args))
	for i, a := range args {
		vals[i] = ser(a, 3, 150)
		types[i] = typeName(a)
	}
	mu.Lock()
	caller := currentCaller()
	stacks[0] = append(stacks[0], name)
	mu.Unlock()
	t0 := time.Now()
	return func() {
		ev := map[string]any{"kind": "call", "name": name, "params": params, "args": vals, "argTypes": types,
			"ms": float64(time.Since(t0).Microseconds()) / 1000, "loc": l, "caller": caller}
		if r := recover(); r != nil {
			ev["err"] = fmt.Sprint(r)
			mu.Lock()
			stacks[0] = stacks[0][:max(0, len(stacks[0])-1)]
			mu.Unlock()
			send(ev)
			panic(r)
		}
		mu.Lock()
		stacks[0] = stacks[0][:max(0, len(stacks[0])-1)]
		mu.Unlock()
		send(ev)
	}
}

// Log records a printf-style line with its source location.
func Log(format string, a ...any) {
	l, _, _ := loc(2)
	mu.Lock()
	caller := currentCaller()
	mu.Unlock()
	send(map[string]any{"kind": "log", "level": "info", "text": fmt.Sprintf(format, a...), "loc": l, "caller": caller})
}

// ---------------------------------------------------------------- HTTP

func headers(h http.Header) map[string]string {
	out := map[string]string{}
	for k, v := range h {
		s := strings.Join(v, ", ")
		switch strings.ToLower(k) {
		case "authorization", "cookie", "set-cookie", "x-api-key":
			if len(s) > 10 {
				s = s[:10] + "…"
			}
		}
		out[strings.ToLower(k)] = s
	}
	return out
}

func bodyText(b []byte, h http.Header) any {
	if len(b) == 0 {
		return nil
	}
	if enc := h.Get("Content-Encoding"); enc != "" && enc != "identity" {
		return fmt.Sprintf("<%s %d bytes>", enc, len(b))
	}
	ct := h.Get("Content-Type")
	if ct != "" && !strings.Contains(ct, "json") && !strings.Contains(ct, "text") && !strings.Contains(ct, "xml") && !strings.Contains(ct, "form") {
		return fmt.Sprintf("<%s %d bytes>", strings.Split(ct, ";")[0], len(b))
	}
	if len(b) > maxBody {
		return string(b[:maxBody]) + "…"
	}
	return string(b)
}

type transport struct{ base http.RoundTripper }

func (t *transport) RoundTrip(req *http.Request) (*http.Response, error) {
	if req.Header.Get("x-cdev-internal") != "" {
		return t.base.RoundTrip(req)
	}
	var reqBody []byte
	if req.Body != nil && req.GetBody != nil {
		if rc, err := req.GetBody(); err == nil {
			reqBody, _ = io.ReadAll(io.LimitReader(rc, maxBody))
			rc.Close()
		}
	}
	host := req.URL.Hostname()
	cid := ""
	if host == "localhost" || host == "127.0.0.1" {
		req = req.Clone(req.Context())
		cid = fmt.Sprintf("%x", time.Now().UnixNano())
		req.Header.Set("x-cdev-from", service)
		req.Header.Set("x-cdev-id", cid)
	}
	mu.Lock()
	caller := currentCaller()
	mu.Unlock()
	t0 := time.Now()
	resp, err := t.base.RoundTrip(req)
	ev := map[string]any{"kind": "http", "dir": "out", "method": req.Method, "url": req.URL.String(), "caller": caller, "cid": cid,
		"ms": float64(time.Since(t0).Microseconds()) / 1000,
		"req": map[string]any{"headers": headers(req.Header), "body": bodyText(reqBody, req.Header)}}
	if err != nil {
		ev["err"] = err.Error()
		send(ev)
		return resp, err
	}
	// peek at the body without consuming it for the caller
	peek, _ := io.ReadAll(io.LimitReader(resp.Body, maxBody))
	resp.Body = struct {
		io.Reader
		io.Closer
	}{io.MultiReader(bytes.NewReader(peek), resp.Body), resp.Body}
	ev["status"] = resp.StatusCode
	ev["res"] = map[string]any{"headers": headers(resp.Header), "body": bodyText(peek, resp.Header)}
	send(ev)
	return resp, nil
}

type recorder struct {
	http.ResponseWriter
	status int
	body   bytes.Buffer
}

func (r *recorder) WriteHeader(code int) { r.status = code; r.ResponseWriter.WriteHeader(code) }
func (r *recorder) Write(b []byte) (int, error) {
	if r.status == 0 {
		r.status = 200
	}
	if r.body.Len() < maxBody {
		r.body.Write(b)
	}
	return r.ResponseWriter.Write(b)
}

// Handler wraps an http.Handler to record every request with bodies.
func Handler(h http.Handler) http.Handler {
	if off {
		return h
	}
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		t0 := time.Now()
		var reqBody []byte
		if r.Body != nil {
			reqBody, _ = io.ReadAll(io.LimitReader(r.Body, maxBody))
			r.Body = io.NopCloser(io.MultiReader(bytes.NewReader(reqBody), r.Body))
		}
		rec := &recorder{ResponseWriter: w}
		route := r.Method + " " + r.URL.Path
		mu.Lock()
		stacks[0] = append(stacks[0], route)
		mu.Unlock()
		defer func() {
			mu.Lock()
			stacks[0] = stacks[0][:max(0, len(stacks[0])-1)]
			mu.Unlock()
			from := r.Header.Get("x-cdev-from")
			if from == "" {
				from = "client"
				if strings.Contains(r.UserAgent(), "Mozilla") {
					from = "browser"
				}
			}
			send(map[string]any{"kind": "http", "dir": "in", "method": r.Method, "url": "http://" + r.Host + r.URL.RequestURI(),
				"from": from, "cid": r.Header.Get("x-cdev-id"), "handler": route, "status": rec.status, "ms": float64(time.Since(t0).Microseconds()) / 1000,
				"req": map[string]any{"headers": headers(r.Header), "body": bodyText(reqBody, r.Header)},
				"res": map[string]any{"headers": headers(w.Header()), "body": bodyText(rec.body.Bytes(), w.Header())}})
		}()
		h.ServeHTTP(rec, r)
	})
}
