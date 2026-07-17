(module
  (import "aos_realm_v0" "write"
    (func $write (param i32 i32 i32) (result i32)))
  (import "aos_realm_v0" "clock-monotonic-ns"
    (func $clock_monotonic_ns (result i64)))
  (import "aos_realm_v0" "exit"
    (func $exit (param i32)))

  (memory (export "memory") 1 1)
  (data (i32.const 0) "hello from AOS Realm\0a")

  (func (export "_start")
    (drop (call $clock_monotonic_ns))
    (drop
      (call $write
        (i32.const 1)
        (i32.const 0)
        (i32.const 21)))
    (call $exit (i32.const 0))
    unreachable))
