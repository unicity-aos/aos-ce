(module
  (import "aos_realm_v0" "arg-len" (func $arg-len (param i32) (result i32)))
  (import "aos_realm_v0" "arg-read"
    (func $arg-read (param i32 i32 i32) (result i32)))
  (import "aos_realm_v0" "write"
    (func $write (param i32 i32 i32) (result i32)))
  (import "aos_realm_v0" "exit" (func $exit (param i32)))
  (memory (export "memory") 1 1)
  (data (i32.const 32768) "\n")
  (func (export "_start")
    (local $length i32)
    (local $offset i32)
    (local $written i32)
    (local.set $length (call $arg-len (i32.const 1)))
    (drop (call $arg-read (i32.const 1) (i32.const 0) (local.get $length)))
    (block $write-done
      (loop $write-loop
        (br_if $write-done
          (i32.ge_u (local.get $offset) (local.get $length)))
        (local.set $written
          (call $write
            (i32.const 1)
            (local.get $offset)
            (i32.sub (local.get $length) (local.get $offset))))
        (local.set $offset
          (i32.add (local.get $offset) (local.get $written)))
        (br $write-loop)))
    (drop (call $write (i32.const 1) (i32.const 32768) (i32.const 1)))
    (call $exit (i32.const 0))
    unreachable))
