.option norvc

.equ SYS_READ, 63
.equ SYS_WRITE, 64
.equ SYS_REBOOT, 142

.section .text
.globl _start
.type _start, @function
_start:
    la a0, boot_marker
    li a1, 16
    call write_stdout

command_loop:
    la a0, ready_marker
    li a1, 10
    call write_stdout

    li a0, 0
    la a1, command_buffer
    li a2, 256
    li a7, SYS_READ
    ecall
    blez a0, command_loop
    mv s0, a0

    la t0, command_count
    ld t1, 0(t0)
    addi t1, t1, 1
    sd t1, 0(t0)

    li t0, 5
    bne s0, t0, check_counter
    la t1, command_buffer
    lw t2, 0(t1)
    li t3, 0x676e6970
    bne t2, t3, check_counter
    lbu t2, 4(t1)
    li t3, 10
    bne t2, t3, check_counter
    la a0, result_begin
    li a1, 10
    call write_stdout
    la a0, pong_result
    li a1, 5
    call write_stdout
    j result_ok

check_counter:
    li t0, 8
    bne s0, t0, check_shutdown
    la t1, command_buffer
    ld t2, 0(t1)
    li t3, 0x0a7265746e756f63
    bne t2, t3, check_shutdown
    la a0, result_begin
    li a1, 10
    call write_stdout
    la a0, counter_prefix
    li a1, 8
    call write_stdout
    call write_counter
    la a0, newline
    li a1, 1
    call write_stdout
    j result_ok

check_shutdown:
    li t0, 9
    bne s0, t0, check_echo
    la t1, command_buffer
    ld t2, 0(t1)
    li t3, 0x6e776f6474756873
    bne t2, t3, check_echo
    lbu t2, 8(t1)
    li t3, 10
    bne t2, t3, check_echo
    la a0, result_begin
    li a1, 10
    call write_stdout
    la a0, shutdown_result
    li a1, 14
    call write_stdout
    la a0, result_ok_marker
    li a1, 10
    call write_stdout
    li a0, 0xfee1dead
    li a1, 0x28121969
    li a2, 0x4321fedc
    li a3, 0
    li a7, SYS_REBOOT
    ecall
    j command_loop

check_echo:
    li t0, 6
    blt s0, t0, unknown_command
    la t1, command_buffer
    lw t2, 0(t1)
    li t3, 0x6f686365
    bne t2, t3, unknown_command
    lbu t2, 4(t1)
    li t3, 32
    bne t2, t3, unknown_command
    la a0, result_begin
    li a1, 10
    call write_stdout
    la a0, command_buffer
    addi a0, a0, 5
    addi a1, s0, -5
    call write_stdout
    j result_ok

unknown_command:
    la a0, result_begin
    li a1, 10
    call write_stdout
    la a0, unknown_result
    li a1, 16
    call write_stdout
    la a0, result_usage_marker
    li a1, 11
    call write_stdout
    j command_loop

result_ok:
    la a0, result_ok_marker
    li a1, 10
    call write_stdout
    j command_loop

write_stdout:
    mv a2, a1
    mv a1, a0
    li a0, 1
    li a7, SYS_WRITE
    ecall
    ret

write_counter:
    la t0, command_count
    ld t0, 0(t0)
    la t1, decimal_buffer_end
    li t2, 10
1:
    remu t3, t0, t2
    addi t3, t3, 48
    addi t1, t1, -1
    sb t3, 0(t1)
    divu t0, t0, t2
    bnez t0, 1b
    mv a0, t1
    la t0, decimal_buffer_end
    sub a1, t0, t1
    j write_stdout
.size _start, . - _start

.section .rodata
boot_marker:
    .ascii "AOS LINUX /init\n"
ready_marker:
    .ascii "AOS READY\n"
result_begin:
    .ascii "AOS BEGIN\n"
result_ok_marker:
    .ascii "AOS END 0\n"
result_usage_marker:
    .ascii "AOS END 64\n"
pong_result:
    .ascii "pong\n"
counter_prefix:
    .ascii "counter="
shutdown_result:
    .ascii "shutting down\n"
unknown_result:
    .ascii "unknown command\n"
newline:
    .ascii "\n"

.section .bss
.balign 8
command_count:
    .zero 8
command_buffer:
    .zero 256
decimal_buffer:
    .zero 32
decimal_buffer_end:
