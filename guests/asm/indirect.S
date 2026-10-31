# Guest without libc for the inline lookup of indirect transfers (k3-03): an interpreter
# that dispatches through a jump table, recursion that returns with `ret` to two call
# sites, an indirect call through a register, and flags that must survive a `ret` and an
# indirect `jmp`. Prints "table 2454267021 fib 6765 call 55" and exits with 0; any failed
# check exits with code 1.
	.intel_syntax noprefix

	.text
	.globl _start
_start:
	# 1000 passes of the program in `ops` over eax: each pass does eax = 8 * eax + 37.
	xor eax, eax
	mov r15d, 1000
	lea rsi, [rip + ops]
next:
	movzx ecx, byte ptr [rsi]
	inc rsi
	lea rdx, [rip + table]
	jmp qword ptr [rdx + rcx*8]
op_add:
	add eax, 3
	jmp next
op_double:
	add eax, eax
	jmp next
op_sub:
	dec eax
	jmp next
op_loop:
	dec r15d
	jz op_end
	lea rsi, [rip + ops]
	jmp next
op_end:
	mov r12d, eax

	# Recursion: fib(20) returns 21891 times to the two call sites inside fib.
	mov edi, 20
	call fib
	mov r13, rax

	# Indirect call through a register.
	lea rax, [rip + fib]
	mov edi, 10
	call rax
	mov r14, rax

	# Flags set before a `ret` are the ones the caller sees.
	mov edi, 5
	call cmp5
	jne fail
	mov edi, 6
	call cmp5
	je fail
	# And across an indirect `jmp`.
	lea rdx, [rip + 1f]
	cmp r14, 55
	jmp rdx
1:	jne fail

	lea rsi, [rip + s_table]
	mov edx, 6
	call print_str
	mov edi, r12d
	call print_num
	lea rsi, [rip + s_fib]
	mov edx, 5
	call print_str
	mov rdi, r13
	call print_num
	lea rsi, [rip + s_call]
	mov edx, 6
	call print_str
	mov rdi, r14
	call print_num
	lea rsi, [rip + s_nl]
	mov edx, 1
	call print_str
	mov eax, 60
	xor edi, edi
	syscall

fail:
	mov eax, 60
	mov edi, 1
	syscall

# rax = fib(rdi), recursively.
fib:
	cmp rdi, 2
	jb 1f
	push rbx
	push rbp
	mov rbp, rdi
	lea rdi, [rbp - 1]
	call fib
	mov rbx, rax
	lea rdi, [rbp - 2]
	call fib
	add rax, rbx
	pop rbp
	pop rbx
	ret
1:	mov rax, rdi
	ret

# ZF = (edi == 5).
cmp5:
	cmp edi, 5
	ret

# write(1, rsi, rdx)
print_str:
	mov eax, 1
	mov edi, 1
	syscall
	ret

# Writes rdi in decimal.
print_num:
	sub rsp, 32
	lea rsi, [rsp + 32]
	mov rax, rdi
	mov ecx, 10
2:	xor edx, edx
	div rcx
	add dl, '0'
	dec rsi
	mov byte ptr [rsi], dl
	test rax, rax
	jnz 2b
	lea rdx, [rsp + 32]
	sub rdx, rsi
	mov eax, 1
	mov edi, 1
	syscall
	add rsp, 32
	ret

	.section .rodata
	.balign 8
table:
	.quad op_add, op_double, op_sub, op_loop
ops:
	.byte 0, 1, 0, 1, 2, 1, 0, 3
s_table:
	.ascii "table "
s_fib:
	.ascii " fib "
s_call:
	.ascii " call "
s_nl:
	.ascii "\n"

	.section .note.GNU-stack, "", @progbits
