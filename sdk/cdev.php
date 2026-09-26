<?php
/**
 * cdev PHP SDK — one file, no extensions or Composer needed.
 *
 *   require 'cdev.php';                    // `cdev run` / `cdev watch` preload it for you
 *   $total = cdev_w($price * $qty);        // records "$price * $qty" = 42 : int, returns the value
 *   cdev_w($tree, 'tree');                 // explicit label; arrays, objects, linked nodes all work
 *   $find = cdev_trace(function (int $id) { ... }, 'findUser');   // args by name, return, errors, duration
 *   cdev_log('hello', $user);
 *
 * Auto: PHP warnings/notices, uncaught exceptions, and — when serving HTTP
 * (php -S, php-fpm) — each request with its body and response.
 * Set CDEV_OFF=1 to disable.
 */

if (!function_exists('cdev_w')) {

final class Cdev
{
    public static $queue = [];
    public static $last = [];
    public static $stack = [];
    public static $off = false;
    public static $svc = 'php';
    public static $host = '127.0.0.1';
    public static $port = 4400;
    public static $cwd = '';
    public static $t0 = 0.0;

    public static function init(): void
    {
        self::$off = (bool) getenv('CDEV_OFF');
        self::$svc = getenv('CDEV_SERVICE') ?: basename(getcwd() ?: 'php');
        self::$cwd = rtrim(getcwd() ?: '', '/') . '/';
        self::$t0 = microtime(true);
        $u = parse_url(getenv('CDEV_URL') ?: 'http://127.0.0.1:4400');
        self::$host = ($u['host'] ?? '127.0.0.1') === 'localhost' ? '127.0.0.1' : ($u['host'] ?? '127.0.0.1');
        self::$port = (int) ($u['port'] ?? 4400);
    }

    public static function send(array $ev): void
    {
        if (self::$off) return;
        $ev['ts'] = microtime(true) * 1000;
        $ev['svc'] = $ev['svc'] ?? self::$svc;
        $ev['lang'] = 'php';
        $ev['pid'] = getmypid();
        self::$queue[] = $ev;
        if (count(self::$queue) >= 200) self::flush();
    }

    public static function flush(): void
    {
        if (!self::$queue) return;
        $body = json_encode(self::$queue, JSON_PARTIAL_OUTPUT_ON_ERROR | JSON_INVALID_UTF8_SUBSTITUTE);
        self::$queue = [];
        $fp = @stream_socket_client('tcp://' . self::$host . ':' . self::$port, $errno, $errstr, 1.0);
        if (!$fp) return;
        stream_set_timeout($fp, 2);
        fwrite($fp, "POST /ingest HTTP/1.1\r\nHost: " . self::$host . "\r\nContent-Type: application/json\r\nContent-Length: " . strlen($body) . "\r\nConnection: close\r\n\r\n" . $body);
        while (!feof($fp) && fread($fp, 256) !== false) {
            $meta = stream_get_meta_data($fp);
            if ($meta['timed_out']) break;
        }
        fclose($fp);
    }

    /** JSON-safe snapshot; repeated objects become {"__ref": id}. */
    public static function ser($v, int $maxDepth = 6, int $maxNodes = 400)
    {
        $seen = [];
        $nodes = 0;
        $walk = function ($v, int $d) use (&$walk, &$seen, &$nodes, $maxDepth, $maxNodes) {
            if ($v === null || is_bool($v) || is_int($v)) return $v;
            if (is_float($v)) return is_finite($v) ? $v : ['__t' => 'float', 'v' => (string) $v];
            if (is_string($v)) return strlen($v) > 2000 ? substr($v, 0, 2000) . '…' : $v;
            if (is_resource($v)) return ['__t' => 'resource', 'v' => get_resource_type($v)];
            if (++$nodes > $maxNodes || $d > $maxDepth) return ['__t' => is_object($v) ? get_class($v) : 'array', 'v' => '…'];
            if ($v instanceof \Closure) return ['__t' => 'function', 'name' => 'Closure'];
            if ($v instanceof \DateTimeInterface) return ['__t' => get_class($v), 'v' => $v->format(DATE_ATOM)];
            if ($v instanceof \Throwable) return ['__t' => get_class($v), 'message' => $v->getMessage()];
            if (is_array($v)) {
                $isList = $v === [] || array_keys($v) === range(0, count($v) - 1);
                $out = [];
                $i = 0;
                foreach ($v as $k => $x) {
                    if ($i++ >= 500) break;
                    if ($isList) $out[] = $walk($x, $d + 1); else $out[(string) $k] = $walk($x, $d + 1);
                }
                return $isList ? $out : (object) $out;
            }
            if (is_object($v)) {
                $id = spl_object_id($v);
                if (isset($seen[$id])) {
                    $o = $seen[$id];
                    if (!isset($o->__id)) $o->__id = $id;
                    return ['__ref' => $id];
                }
                $o = new \stdClass();
                $seen[$id] = $o;
                $o->__class = (new \ReflectionClass($v))->getShortName();
                // (array) cast exposes private/protected props as "\0Class\0name" / "\0*\0name"
                foreach ((array) $v as $k => $x) {
                    $name = ($p = strrpos($k, "\0")) !== false ? substr($k, $p + 1) : $k;
                    $link = in_array($name, ['next', 'prev', 'left', 'right', 'children'], true);
                    $o->$name = $walk($x, $link ? $d : $d + 1);
                }
                return $o;
            }
            return ['__t' => gettype($v)];
        };
        return $walk($v, 0);
    }

    public static function loc(array $frame): string
    {
        $f = $frame['file'] ?? '?';
        if (strpos($f, self::$cwd) === 0) $f = substr($f, strlen(self::$cwd));
        return $f . ':' . ($frame['line'] ?? 0);
    }

    /** Source text of the first argument of `fn(` on the caller's line. */
    public static function label(array $frame, string $fn): string
    {
        $src = @file($frame['file'] ?? '');
        if (!$src) return '';
        $line = $src[($frame['line'] ?? 1) - 1] ?? '';
        $i = strpos($line, $fn . '(');
        if ($i === false) return '';
        $depth = 0;
        $out = '';
        foreach (str_split(substr($line, $i + strlen($fn) + 1)) as $c) {
            if (strpos('([{', $c) !== false) $depth++;
            if (strpos(')]}', $c) !== false) { if ($depth === 0) break; $depth--; }
            if ($c === ',' && $depth === 0) break;
            $out .= $c;
        }
        return substr(trim($out), 0, 60);
    }

    public static function caller()
    {
        return self::$stack ? end(self::$stack) : null;
    }
}

Cdev::init();

/** Record a value (+ inferred type, source line) and return it unchanged. */
function cdev_w($value, ?string $label = null)
{
    if (Cdev::$off) return $value;
    $frame = debug_backtrace(DEBUG_BACKTRACE_IGNORE_ARGS, 1)[0];
    $loc = Cdev::loc($frame);
    $name = $label ?? (Cdev::label($frame, 'cdev_w') ?: $loc);
    $v = Cdev::ser($value, 64, 2000);
    $key = json_encode($v);
    if ((Cdev::$last[$loc . $name] ?? null) === $key) return $value;
    Cdev::$last[$loc . $name] = $key;
    Cdev::send(['kind' => 'watch', 'name' => $name, 'v' => $v, 'loc' => $loc, 'caller' => Cdev::caller()]);
    return $value;
}

/** Wrap a callable: records params by name, args, return value, exceptions, duration. */
function cdev_trace(callable $fn, ?string $name = null): callable
{
    if (Cdev::$off) return $fn;
    $ref = is_array($fn) ? new \ReflectionMethod($fn[0], $fn[1]) : new \ReflectionFunction(\Closure::fromCallable($fn));
    $name = $name ?? ($ref->getName() === '{closure}' ? 'closure' : $ref->getName());
    $params = array_map(fn($p) => $p->getName(), $ref->getParameters());
    $loc = Cdev::loc(['file' => $ref->getFileName(), 'line' => $ref->getStartLine()]);
    return function (...$args) use ($fn, $name, $params, $loc) {
        $caller = Cdev::caller();
        $ev = ['kind' => 'call', 'name' => $name, 'params' => array_slice($params, 0, count($args)),
               'args' => array_map(fn($a) => Cdev::ser($a, 3, 150), $args), 'loc' => $loc, 'caller' => $caller];
        Cdev::$stack[] = $name;
        $t0 = hrtime(true);
        try {
            $ret = $fn(...$args);
            $ev['ret'] = Cdev::ser($ret, 3, 150);
            return $ret;
        } catch (\Throwable $e) {
            $ev['err'] = get_class($e) . ': ' . $e->getMessage();
            throw $e;
        } finally {
            array_pop(Cdev::$stack);
            $ev['ms'] = round((hrtime(true) - $t0) / 1e6, 3);
            Cdev::send($ev);
        }
    };
}

function cdev_log(...$args): void
{
    $frame = debug_backtrace(DEBUG_BACKTRACE_IGNORE_ARGS, 1)[0];
    $text = implode(' ', array_map(fn($a) => is_string($a) ? $a : json_encode(Cdev::ser($a, 3, 100)), $args));
    Cdev::send(['kind' => 'log', 'level' => 'info', 'text' => $text, 'args' => array_map(fn($a) => Cdev::ser($a, 3, 150), $args),
                'loc' => Cdev::loc($frame), 'caller' => Cdev::caller()]);
}

// ---------------------------------------------------------------- auto hooks

if (!Cdev::$off) {
    $prevError = set_error_handler(function ($no, $str, $file, $line) use (&$prevError) {
        $level = in_array($no, [E_WARNING, E_USER_WARNING, E_DEPRECATED, E_USER_DEPRECATED], true) ? 'warn'
            : (in_array($no, [E_NOTICE, E_USER_NOTICE], true) ? 'info' : 'error');
        Cdev::send(['kind' => 'log', 'level' => $level, 'text' => $str, 'loc' => Cdev::loc(['file' => $file, 'line' => $line])]);
        return $prevError ? $prevError($no, $str, $file, $line) : false; // false: PHP's normal handling continues
    });

    $prevException = set_exception_handler(function (\Throwable $e) use (&$prevException) {
        Cdev::send(['kind' => 'error', 'msg' => get_class($e) . ': ' . $e->getMessage(), 'stack' => $e->getTraceAsString(),
                    'loc' => Cdev::loc(['file' => $e->getFile(), 'line' => $e->getLine()])]);
        Cdev::flush();
        if ($prevException) { $prevException($e); return; }
        fwrite(STDERR, "PHP Fatal error:  Uncaught " . $e . "\n");
        exit(255);
    });

    $isWeb = PHP_SAPI !== 'cli';
    if ($isWeb) {
        ob_start(function ($buf) { Cdev::$last['__body'] = (Cdev::$last['__body'] ?? '') . substr($buf, 0, 16384); return $buf; });
        Cdev::$stack[] = ($_SERVER['REQUEST_METHOD'] ?? 'GET') . ' ' . parse_url($_SERVER['REQUEST_URI'] ?? '/', PHP_URL_PATH);
    }

    register_shutdown_function(function () use ($isWeb) {
        $err = error_get_last();
        if ($err && in_array($err['type'], [E_ERROR, E_PARSE, E_CORE_ERROR, E_COMPILE_ERROR], true)) {
            Cdev::send(['kind' => 'error', 'msg' => $err['message'], 'loc' => Cdev::loc($err)]);
        }
        if ($isWeb) {
            if (ob_get_level() > 0) @ob_end_flush();
            $reqHeaders = [];
            foreach ($_SERVER as $k => $v) {
                if (strpos($k, 'HTTP_') === 0) $reqHeaders[strtolower(str_replace('_', '-', substr($k, 5)))] = (string) $v;
            }
            $resHeaders = [];
            foreach (headers_list() as $h) { [$k, $v] = array_pad(explode(':', $h, 2), 2, ''); $resHeaders[strtolower($k)] = trim($v); }
            $ua = $reqHeaders['user-agent'] ?? '';
            Cdev::send([
                'kind' => 'http', 'dir' => 'in', 'method' => $_SERVER['REQUEST_METHOD'] ?? 'GET',
                'url' => 'http://' . ($_SERVER['HTTP_HOST'] ?? 'localhost') . ($_SERVER['REQUEST_URI'] ?? '/'),
                'from' => $reqHeaders['x-cdev-from'] ?? (strpos($ua, 'Mozilla') !== false ? 'browser' : 'client'),
                'cid' => $reqHeaders['x-cdev-id'] ?? null,
                'status' => http_response_code() ?: 200,
                'ms' => round((microtime(true) - ($_SERVER['REQUEST_TIME_FLOAT'] ?? Cdev::$t0)) * 1000, 2),
                'req' => ['headers' => $reqHeaders, 'body' => substr((string) file_get_contents('php://input'), 0, 16384) ?: null],
                'res' => ['headers' => $resHeaders, 'body' => Cdev::$last['__body'] ?? null],
            ]);
            if (!empty($_SERVER['SERVER_PORT'])) Cdev::send(['kind' => 'service', 'name' => Cdev::$svc, 'port' => (int) $_SERVER['SERVER_PORT']]);
        }
        Cdev::flush();
    });
}

} // function_exists guard
