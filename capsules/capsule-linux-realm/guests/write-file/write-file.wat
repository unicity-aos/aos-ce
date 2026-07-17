(module
  (import "aos_realm_v0" "arg-len" (func $arg-len (param i32) (result i32)))
  (import "aos_realm_v0" "arg-read"
    (func $arg-read (param i32 i32 i32) (result i32)))
  (import "aos_realm_v0" "open"
    (func $open (param i32 i32 i32) (result i32)))
  (import "aos_realm_v0" "write"
    (func $write (param i32 i32 i32) (result i32)))
  (import "aos_realm_v0" "close" (func $close (param i32) (result i32)))
  (import "aos_realm_v0" "exit" (func $exit (param i32)))
  (memory (export "memory") 1 1)
  (func (export "_start")
    (local $path-length i32)
    (local $data-length i32)
    (local $fd i32)
    (local.set $path-length (call $arg-len (i32.const 1)))
    (local.set $data-length (call $arg-len (i32.const 2)))
    (drop (call $arg-read (i32.const 1) (i32.const 0) (local.get $path-length)))
    (drop (call $arg-read
      (i32.const 2)
      (i32.const 4096)
      (local.get $data-length)))
    (local.set $fd
      (call $open (i32.const 0) (local.get $path-length) (i32.const 1)))
    (drop (call $write
      (local.get $fd)
      (i32.const 4096)
      (local.get $data-length)))
    (drop (call $close (local.get $fd)))
    (call $exit (i32.const 0))
    unreachable))
