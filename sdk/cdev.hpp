// cdev C++ SDK — header-only, C++17, POSIX (macOS / Linux).
//
//   #include "cdev.hpp"
//   int total = CDEV_W(price * qty);     // records "price * qty" = 42 : int, returns the value
//   CDEV_W(grid);                        // std::vector<std::vector<int>> shows as a matrix
//   void solve(int n, const std::string& s) {
//     CDEV_TRACE(n, s);                  // args + duration + caller→callee, recorded at scope exit
//     CDEV_LOG("n is ", n);
//   }
//
// Build with -DCDEV_DISABLE to compile every macro away. Runtime: CDEV_OFF=1.
// Events go to CDEV_URL (default http://127.0.0.1:4400) from a background thread.
#pragma once

#ifdef CDEV_DISABLE
#define CDEV_W(x) (x)
#define CDEV_WATCH(name, x) (x)
#define CDEV_LOG(...) ((void)0)
#define CDEV_TRACE(...) ((void)0)
#else

#include <arpa/inet.h>
#include <atomic>
#include <chrono>
#include <condition_variable>
#include <cstdlib>
#include <cstring>
#include <cxxabi.h>
#include <deque>
#include <map>
#include <memory>
#include <mutex>
#include <netdb.h>
#include <netinet/in.h>
#include <optional>
#include <sstream>
#include <string>
#include <sys/socket.h>
#include <thread>
#include <type_traits>
#include <typeinfo>
#include <unistd.h>
#include <unordered_map>
#include <utility>
#include <vector>

namespace cdev {

// ---------- JSON helpers ----------
inline std::string esc(const std::string& s) {
  std::string o = "\"";
  for (unsigned char c : s) {
    switch (c) {
      case '"': o += "\\\""; break;
      case '\\': o += "\\\\"; break;
      case '\n': o += "\\n"; break;
      case '\r': o += "\\r"; break;
      case '\t': o += "\\t"; break;
      default:
        if (c < 0x20) { char b[8]; std::snprintf(b, sizeof b, "\\u%04x", c); o += b; }
        else o += static_cast<char>(c);
    }
  }
  return o + "\"";
}

inline void replace_all(std::string& s, const std::string& a, const std::string& b) {
  for (size_t p = 0; (p = s.find(a, p)) != std::string::npos; p += b.size()) s.replace(p, a.size(), b);
}

template <class T> std::string type_name() {
  int st = 0;
  const char* raw = typeid(T).name();
  std::unique_ptr<char, void (*)(void*)> d(abi::__cxa_demangle(raw, nullptr, nullptr, &st), std::free);
  std::string s = st == 0 ? d.get() : raw;
  if (std::is_const_v<std::remove_reference_t<T>>) s = "const " + s;
  replace_all(s, "std::__1::", "std::");
  replace_all(s, "std::__cxx11::", "std::");
  replace_all(s, "std::basic_string<char, std::char_traits<char>, std::allocator<char> >", "std::string");
  replace_all(s, "std::basic_string<char, std::char_traits<char>, std::allocator<char>>", "std::string");
  // drop default allocator/comparator template args
  for (const char* junk : {", std::allocator<", ", std::less<", ", std::hash<", ", std::equal_to<", ", std::default_delete<"}) {
    size_t p;
    while ((p = s.find(junk)) != std::string::npos) {
      int depth = 0; size_t i = p + 2;
      for (; i < s.size(); ++i) {
        if (s[i] == '<') depth++;
        else if (s[i] == '>') { if (--depth == 0) { ++i; break; } }
      }
      s.erase(p, i - p);
    }
  }
  replace_all(s, " >", ">");
  return s;
}

// ---------- value → JSON (containers recurse; streamables print; else type name) ----------
template <class T, class = void> struct is_iterable : std::false_type {};
template <class T> struct is_iterable<T, std::void_t<decltype(std::begin(std::declval<const T&>())), decltype(std::end(std::declval<const T&>()))>> : std::true_type {};
template <class T, class = void> struct is_streamable : std::false_type {};
template <class T> struct is_streamable<T, std::void_t<decltype(std::declval<std::ostream&>() << std::declval<const T&>())>> : std::true_type {};
template <class T, class = void> struct is_mapping : std::false_type {};
template <class T> struct is_mapping<T, std::void_t<typename T::key_type, typename T::mapped_type>> : std::true_type {};
template <class T> struct is_pair : std::false_type {};
template <class A, class B> struct is_pair<std::pair<A, B>> : std::true_type {};
template <class T> struct is_optional : std::false_type {};
template <class A> struct is_optional<std::optional<A>> : std::true_type {};

template <class T> std::string to_json(const T& v, int depth = 0);

template <class T> std::string to_json(const T& v, int depth) {
  using U = std::decay_t<T>;
  if (depth > 8) return "\"…\"";
  if constexpr (std::is_same_v<U, bool>) return v ? "true" : "false";
  else if constexpr (std::is_same_v<U, char>) return esc(std::string(1, v));
  else if constexpr (std::is_arithmetic_v<U>) { std::ostringstream o; o << +v; return o.str(); }
  else if constexpr (std::is_same_v<U, std::string> || std::is_same_v<U, const char*> || std::is_same_v<U, char*>) return esc(std::string(v));
  else if constexpr (std::is_convertible_v<U, std::string_view>) return esc(std::string(std::string_view(v)));
  else if constexpr (std::is_pointer_v<U>) {
    if (!v) return "null";
    std::ostringstream o; o << static_cast<const void*>(v); return esc(o.str());
  }
  else if constexpr (is_optional<U>::value) return v ? to_json(*v, depth) : "null";
  else if constexpr (is_pair<U>::value) return "[" + to_json(v.first, depth + 1) + "," + to_json(v.second, depth + 1) + "]";
  else if constexpr (is_mapping<U>::value) {
    std::string o = "{\"__t\":\"Map\",\"size\":" + std::to_string(v.size()) + ",\"entries\":[";
    size_t n = 0;
    for (const auto& [k, x] : v) {
      if (n++) o += ",";
      if (n > 100) break;
      o += "[" + to_json(k, depth + 1) + "," + to_json(x, depth + 1) + "]";
    }
    return o + "]}";
  }
  else if constexpr (is_iterable<U>::value) {
    std::string o = "[";
    size_t n = 0;
    for (const auto& x : v) {
      if (n) o += ",";
      if (++n > 500) { o += "{\"__t\":\"more\"}"; break; }
      o += to_json(x, depth + 1);
    }
    return o + "]";
  }
  else if constexpr (is_streamable<U>::value) { std::ostringstream o; o << v; return esc(o.str()); }
  else return "{\"__t\":" + esc(type_name<U>()) + "}";
}

// ---------- transport ----------
class Client {
 public:
  Client() {
    off_ = std::getenv("CDEV_OFF") != nullptr;
    std::string url = std::getenv("CDEV_URL") ? std::getenv("CDEV_URL") : "http://127.0.0.1:4400";
    auto p = url.find("://");
    std::string hp = p == std::string::npos ? url : url.substr(p + 3);
    hp = hp.substr(0, hp.find('/'));
    auto c = hp.rfind(':');
    host_ = c == std::string::npos ? hp : hp.substr(0, c);
    port_ = c == std::string::npos ? 80 : std::atoi(hp.c_str() + c + 1);
    if (host_ == "localhost") host_ = "127.0.0.1";
    const char* svc = std::getenv("CDEV_SERVICE");
    svc_ = svc ? svc : "cpp";
    if (!off_) worker_ = std::thread([this] { loop(); });
  }
  ~Client() {
    { std::lock_guard<std::mutex> g(m_); stop_ = true; }
    cv_.notify_all();
    if (worker_.joinable()) worker_.join();
  }
  bool off() const { return off_; }
  const std::string& svc() const { return svc_; }

  void send(std::string fields) {
    if (off_) return;
    using namespace std::chrono;
    double ts = duration_cast<microseconds>(system_clock::now().time_since_epoch()).count() / 1000.0;
    std::ostringstream o;
    o.precision(15);
    o << "{\"ts\":" << ts << ",\"svc\":" << esc(svc_) << ",\"lang\":\"cpp\",\"pid\":" << getpid() << "," << fields << "}";
    std::lock_guard<std::mutex> g(m_);
    q_.push_back(o.str());
    if (q_.size() > 5000) q_.pop_front();
    cv_.notify_one();
  }

 private:
  void loop() {
    std::unique_lock<std::mutex> lk(m_);
    while (true) {
      cv_.wait_for(lk, std::chrono::milliseconds(100), [this] { return stop_ || !q_.empty(); });
      if (q_.empty()) { if (stop_) return; continue; }
      std::string body = "[";
      for (size_t i = 0; i < q_.size(); ++i) body += (i ? "," : "") + q_[i];
      body += "]";
      q_.clear();
      bool last = stop_;
      lk.unlock();
      post(body);
      lk.lock();
      if (last && q_.empty()) return;
    }
  }

  void post(const std::string& body) {
    int fd = ::socket(AF_INET, SOCK_STREAM, 0);
    if (fd < 0) return;
#ifdef SO_NOSIGPIPE
    int one = 1;
    setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &one, sizeof one);
#endif
#ifdef MSG_NOSIGNAL
    const int flags = MSG_NOSIGNAL;
#else
    const int flags = 0;
#endif
    sockaddr_in a{};
    a.sin_family = AF_INET;
    a.sin_port = htons(static_cast<uint16_t>(port_));
    inet_pton(AF_INET, host_.c_str(), &a.sin_addr);
    if (::connect(fd, reinterpret_cast<sockaddr*>(&a), sizeof a) == 0) {
      std::string req = "POST /ingest HTTP/1.1\r\nHost: " + host_ + "\r\nContent-Type: application/json\r\nContent-Length: " +
                        std::to_string(body.size()) + "\r\nConnection: close\r\n\r\n" + body;
      size_t sent = 0;
      while (sent < req.size()) {
        ssize_t n = ::send(fd, req.data() + sent, req.size() - sent, flags);
        if (n <= 0) break;
        sent += static_cast<size_t>(n);
      }
      char buf[256];
      while (::recv(fd, buf, sizeof buf, 0) > 0) {}
    }
    ::close(fd);
  }

  std::string host_, svc_;
  int port_ = 4400;
  bool off_ = false, stop_ = false;
  std::deque<std::string> q_;
  std::mutex m_;
  std::condition_variable cv_;
  std::thread worker_;
};

inline Client& client() {
  static Client c;
  return c;
}

inline std::string loc(const char* file, int line) {
  std::string f = file;
  return esc(f + ":" + std::to_string(line));
}

inline std::vector<std::string>& call_stack() {
  thread_local std::vector<std::string> s;
  return s;
}

inline std::string caller_json() {
  auto& s = call_stack();
  return s.empty() ? "null" : esc(s.back());
}

// ---------- memory: where a value lives, what it points at ----------
inline std::string hex(const void* p) {
  char b[32];
  std::snprintf(b, sizeof b, "0x%llx", static_cast<unsigned long long>(reinterpret_cast<uintptr_t>(p)));
  return b;
}

inline char& static_marker() { static char c; return c; }

/// stack / heap / static, by distance from the current stack pointer and a static variable.
inline const char* region_of(const void* p) {
  volatile char probe = 0;
  auto sp = reinterpret_cast<uintptr_t>(&probe);
  auto a = reinterpret_cast<uintptr_t>(p);
  if (a + 65536 >= sp && a < sp + (8u << 20)) return "stack";
  auto st = reinterpret_cast<uintptr_t>(&static_marker());
  if (a + (64u << 20) > st && a < st + (64u << 20)) return "static";
  return "heap";
}

template <class T, class = void> struct has_buffer : std::false_type {};
template <class T> struct has_buffer<T, std::void_t<decltype(std::declval<const T&>().data()), decltype(std::declval<const T&>().capacity())>> : std::true_type {};
template <class T, class = void> struct is_smart_ptr : std::false_type {};
template <class T> struct is_smart_ptr<T, std::void_t<typename T::element_type, decltype(std::declval<const T&>().get())>> : std::true_type {};
template <class T, class = void> struct has_use_count : std::false_type {};
template <class T> struct has_use_count<T, std::void_t<decltype(std::declval<const T&>().use_count())>> : std::true_type {};

/// {"addr","size","region", "ptr"?, "target_size"?, "target_region"?, "heap"?:{addr,len,cap,elem,region,inline}, "refs"?}
template <class T> std::string mem_json(const T& v) {
  using U = std::decay_t<T>;
  std::string o = "{\"addr\":" + esc(hex(&v)) + ",\"size\":" + std::to_string(sizeof(U)) + ",\"region\":" + esc(region_of(&v));
  if constexpr (std::is_pointer_v<U>) {
    o += ",\"ptr\":" + (v ? esc(hex(v)) : std::string("null"));
    if constexpr (!std::is_void_v<std::remove_cv_t<std::remove_pointer_t<U>>> && !std::is_function_v<std::remove_pointer_t<U>>) {
      o += ",\"target_size\":" + std::to_string(sizeof(std::remove_pointer_t<U>));
    }
    if (v) o += ",\"target_region\":" + esc(region_of(v));
  } else if constexpr (is_smart_ptr<U>::value) {
    auto p = v.get();
    o += ",\"ptr\":" + (p ? esc(hex(p)) : std::string("null"));
    if constexpr (!std::is_void_v<typename U::element_type>) o += ",\"target_size\":" + std::to_string(sizeof(typename U::element_type));
    if (p) o += ",\"target_region\":" + esc(region_of(p));
    if constexpr (has_use_count<U>::value) o += ",\"refs\":" + std::to_string(v.use_count());
  } else if constexpr (has_buffer<U>::value) {
    const void* d = static_cast<const void*>(v.data());
    auto self = reinterpret_cast<uintptr_t>(&v), dp = reinterpret_cast<uintptr_t>(d);
    bool inside = dp >= self && dp < self + sizeof(U);  // small-string optimisation: chars stored in the object itself
    using E = std::decay_t<decltype(*v.data())>;
    o += ",\"heap\":{\"addr\":" + esc(hex(d)) + ",\"len\":" + std::to_string(v.size()) + ",\"cap\":" + std::to_string(v.capacity()) +
         ",\"elem\":" + std::to_string(sizeof(E)) + ",\"region\":" + esc(inside ? "inline" : region_of(d)) + "}";
  }
  return o + "}";
}

template <class T> const T& watch(const T& v, const char* name, const char* file, int line) {
  if (client().off()) return v;
  static thread_local std::unordered_map<std::string, std::string> last;
  std::string json = to_json(v);
  std::string mem = mem_json(v);
  std::string key = std::string(file) + ":" + name;
  auto it = last.find(key);
  if (it != last.end() && it->second == json + mem) return v;  // a move or reallocation counts as a change
  last[key] = json + mem;
  client().send("\"kind\":\"watch\",\"name\":" + esc(name) + ",\"v\":" + json + ",\"t\":" + esc(type_name<T>()) +
                ",\"mem\":" + mem + ",\"loc\":" + loc(file, line) + ",\"caller\":" + caller_json());
  return v;
}

template <class... A> void log(const char* file, int line, const A&... a) {
  if (client().off()) return;
  std::ostringstream o;
  auto one = [&](const auto& x) {
    using U = std::decay_t<decltype(x)>;
    if constexpr (is_streamable<U>::value) o << x; else o << to_json(x);
  };
  (one(a), ...);
  client().send("\"kind\":\"log\",\"level\":\"info\",\"text\":" + esc(o.str()) + ",\"loc\":" + loc(file, line) + ",\"caller\":" + caller_json());
}

inline std::vector<std::string> split_names(const char* s) {
  std::vector<std::string> out;
  std::string cur;
  int depth = 0;
  for (const char* p = s; *p; ++p) {
    char c = *p;
    if (c == '(' || c == '<' || c == '[' || c == '{') depth++;
    if (c == ')' || c == '>' || c == ']' || c == '}') depth--;
    if (c == ',' && depth == 0) { out.push_back(cur); cur.clear(); continue; }
    if (!(cur.empty() && c == ' ')) cur += c;
  }
  if (!cur.empty()) out.push_back(cur);
  return out;
}

/// Scope guard: records one `call` event (args, types, duration, caller) when the scope ends.
class Scope {
 public:
  template <class... A>
  Scope(const char* fn, const char* file, int line, const char* names, const A&... a) : fn_(fn), loc_(loc(file, line)) {
    if (client().off()) return;
    caller_ = caller_json();
    auto ns = split_names(names);
    std::vector<std::string> vals{to_json(a)...};
    std::vector<std::string> types{type_name<A>()...};
    params_ = "[";
    args_ = "[";
    types_ = "[";
    for (size_t i = 0; i < vals.size(); ++i) {
      const char* sep = i ? "," : "";
      params_ += sep + esc(i < ns.size() ? ns[i] : "");
      args_ += sep + vals[i];
      types_ += sep + esc(types[i]);
    }
    params_ += "]"; args_ += "]"; types_ += "]";
    call_stack().push_back(fn_);
    t0_ = std::chrono::steady_clock::now();
  }
  ~Scope() {
    if (client().off()) return;
    call_stack().pop_back();
    double ms = std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - t0_).count();
    std::ostringstream o;
    o << "\"kind\":\"call\",\"name\":" << esc(fn_) << ",\"params\":" << params_ << ",\"args\":" << args_
      << ",\"argTypes\":" << types_ << ",\"ms\":" << ms << ",\"loc\":" << loc_ << ",\"caller\":" << caller_;
    if (std::uncaught_exceptions() > 0) o << ",\"err\":\"exception\"";
    client().send(o.str());
  }

 private:
  std::string fn_, loc_, caller_ = "null", params_, args_, types_;
  std::chrono::steady_clock::time_point t0_;
};

}  // namespace cdev

#define CDEV_W(x) (::cdev::watch((x), #x, __FILE__, __LINE__))
#define CDEV_WATCH(name, x) (::cdev::watch((x), name, __FILE__, __LINE__))
#define CDEV_LOG(...) ::cdev::log(__FILE__, __LINE__, __VA_ARGS__)
#define CDEV_CAT2(a, b) a##b
#define CDEV_CAT(a, b) CDEV_CAT2(a, b)
#define CDEV_TRACE(...) ::cdev::Scope CDEV_CAT(cdev_scope_, __LINE__)(__func__, __FILE__, __LINE__, #__VA_ARGS__, ##__VA_ARGS__)

#endif  // CDEV_DISABLE
