# Guest without libc for the first translated path (k1-03): a loop closed by a jcc,
# flags live across a block boundary, call/ret, push/pop, a RIP-relative load, an
# indirect jump through a table, write and exit with code 42. Any failed check exits
# with code 1 before writing anything.
	.intel_syntax noprefix
	.equ MSG_LEN, 19

	.text
	.globl _start
_start:
	# 1 + 2 + ... + 10 with a backward jne.
	xor eax, eax
	mov ecx, 10
1:	add eax, ecx
	dec ecx
	jnz 1b
	# The flags of this cmp must survive the jump into the next block.
	cmp eax, 55
	jmp 2f
2:	jne fail

	# call/ret and push/pop.
	push rax
	mov edi, 7
	call triple
	pop rbx
	cmp rbx, 55
	jne fail
	cmp rax, 21
	jne fail

	# RIP-relative load.
	mov rdx, qword ptr [rip + magic]
	movabs rsi, 0x1122334455667788
	cmp rdx, rsi
	jne fail

	# Indirect jump through a table.
	lea r8, [rip + table]
	mov r9d, 2
	jmp qword ptr [r8 + r9*8]
case0:
case1:
	jmp fail
case2:
	mov eax, 1
	mov edi, 1
	lea rsi, [rip + msg]
	mov edx, MSG_LEN
	syscall
	cmp rax, MSG_LEN
	jne fail
	mov eax, 60
	mov edi, 42
	syscall

triple:
	lea rax, [rdi + rdi*2]
	ret

fail:
	mov eax, 60
	mov edi, 1
	syscall

	.section .rodata
	.balign 8
magic:
	.quad 0x1122334455667788
table:
	.quad case0, case1, case2
msg:
	.ascii "calco jit basic ok\n"
	.if . - msg - MSG_LEN
	.error "MSG_LEN does not match msg"
	.endif

	.section .note.GNU-stack, "", @progbits
