(module
  (import "aos_realm_v0" "read"
    (func $read (param i32 i32 i32) (result i32)))
  (import "aos_realm_v0" "write"
    (func $write (param i32 i32 i32) (result i32)))
  (import "aos_realm_v0" "exit" (func $exit (param i32)))
  (memory (export "memory") 1 1)
  (func (export "_start")
    (local $read-length i32)
    (local $offset i32)
    (local $written i32)
    (block $eof
      (loop $read-loop
        (local.set $read-length
          (call $read (i32.const 0) (i32.const 0) (i32.const 4096)))
        (br_if $eof (i32.eqz (local.get $read-length)))
        (local.set $offset (i32.const 0))
        (block $write-done
          (loop $write-loop
            (br_if $write-done
              (i32.ge_u (local.get $offset) (local.get $read-length)))
            (local.set $written
              (call $write
                (i32.const 1)
                (local.get $offset)
                (i32.sub (local.get $read-length) (local.get $offset))))
            (local.set $offset
              (i32.add (local.get $offset) (local.get $written)))
            (br $write-loop)))
        (br $read-loop)))
    (call $exit (i32.const 0))
    unreachable))
