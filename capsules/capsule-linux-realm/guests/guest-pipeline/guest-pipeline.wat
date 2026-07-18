(module
  (import "aos_realm_v0" "arg-len"
    (func $arg-len (param i32) (result i32)))
  (import "aos_realm_v0" "arg-read"
    (func $arg-read (param i32 i32 i32) (result i32)))
  (import "aos_realm_v0" "pipe"
    (func $pipe (param i32 i32) (result i32)))
  (import "aos_realm_v0" "spawn-signed"
    (func $spawn-signed (param i32 i32 i32 i32 i32 i32) (result i32)))
  (import "aos_realm_v0" "close"
    (func $close (param i32) (result i32)))
  (import "aos_realm_v0" "wait"
    (func $wait (param i32 i32) (result i32)))
  (import "aos_realm_v0" "exit" (func $exit (param i32)))

  (memory (export "memory") 1 1)

  ;; Memory records:
  ;;   0..8    pipe read/write descriptors
  ;;   16..32  consumer generation/PID handle
  ;;   32..48  producer generation/PID handle
  ;;   48..56  producer termination kind/value
  ;;   56..64  consumer termination kind/value
  ;;   1024..  pipeline message copied from argv[1]
  (func (export "_start")
    (local $arg-length i32)
    (local $read-fd i32)
    (local $write-fd i32)

    i32.const 1
    call $arg-len
    local.tee $arg-length
    i32.const 32768
    i32.gt_u
    if
      i32.const 10
      call $exit
    end

    i32.const 1
    i32.const 1024
    local.get $arg-length
    call $arg-read
    local.get $arg-length
    i32.ne
    if
      i32.const 11
      call $exit
    end

    i32.const 4
    i32.const 0
    call $pipe
    i32.eqz
    if
    else
      i32.const 12
      call $exit
    end
    i32.const 0
    i32.load
    local.set $read-fd
    i32.const 4
    i32.load
    local.set $write-fd

    ;; Catalog program 2 is stdin-cat. It inherits our pipe reader as stdin.
    i32.const 2
    i32.const 0
    i32.const 0
    local.get $read-fd
    i32.const 0
    i32.const 16
    call $spawn-signed
    i32.eqz
    if
    else
      i32.const 13
      call $exit
    end

    ;; Catalog program 1 is echo. It receives argv[1] and writes into the pipe.
    i32.const 1
    i32.const 1024
    local.get $arg-length
    local.get $write-fd
    i32.const 1
    i32.const 32
    call $spawn-signed
    i32.eqz
    if
    else
      i32.const 14
      call $exit
    end

    local.get $read-fd
    call $close
    drop
    local.get $write-fd
    call $close
    drop

    i32.const 32
    i32.const 48
    call $wait
    drop
    i32.const 48
    i32.load
    i32.eqz
    i32.const 52
    i32.load
    i32.eqz
    i32.and
    if
    else
      i32.const 15
      call $exit
    end

    i32.const 16
    i32.const 56
    call $wait
    drop
    i32.const 56
    i32.load
    i32.eqz
    i32.const 60
    i32.load
    i32.eqz
    i32.and
    if
      i32.const 0
      call $exit
    else
      i32.const 16
      call $exit
    end))
