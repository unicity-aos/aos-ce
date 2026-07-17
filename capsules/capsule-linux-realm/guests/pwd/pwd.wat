(module
  (import "aos_realm_v0" "cwd-read"
    (func $cwd-read (param i32 i32) (result i32)))
  (import "aos_realm_v0" "write"
    (func $write (param i32 i32 i32) (result i32)))
  (import "aos_realm_v0" "exit" (func $exit (param i32)))
  (memory (export "memory") 1 1)
  (data (i32.const 4096) "\n")
  (func (export "_start") (local $length i32)
    (local.set $length (call $cwd-read (i32.const 0) (i32.const 4096)))
    (drop (call $write (i32.const 1) (i32.const 0) (local.get $length)))
    (drop (call $write (i32.const 1) (i32.const 4096) (i32.const 1)))
    (call $exit (i32.const 0))
    unreachable))
