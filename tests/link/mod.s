| Assembly side of the link test, in the syntax of the GNU assembler.
	.globl	asm_add,asm_sum_table,asm_table,asm_buffer,asm_counter,asm_call_c
	.globl	_start

	.text
_start:
	lea	stack_top,%sp
	jsr	main
halt:	bra.s	halt

asm_add:
	move.w	6(%sp),%d0
	add.w	10(%sp),%d0
	rts

asm_sum_table:
	lea	asm_table(%pc),%a0
	moveq	#0,%d0
	moveq	#4-1,%d1
loop:	moveq	#0,%d2
	move.w	(%a0)+,%d2
	add.l	%d2,%d0
	dbra	%d1,loop
	add.l	c_value,%d0
	add.w	c_words+2,%d0
	rts

asm_call_c:
	move.l	asm_counter,-(%sp)
	jsr	c_function
	addq.l	#4,%sp
	move.l	%d0,asm_counter
	move.l	#asm_buffer+4,%a0
	move.b	%d0,(%a0)
	lea	c_words,%a1
	move.w	4(%a1),asm_buffer+8
	bsr.w	asm_add
	bsr.w	local_helper
	rts

local_helper:
	move.l	#asm_table,%d0
	rts

	.section .rodata
asm_table:
	.word	0x1111,0x2222,0x3333,0x4444
pointers:
	.long	asm_add,local_helper,c_function,asm_buffer+2,c_words+4

	.data
asm_counter:
	.long	5
	.long	pointers

	.bss
asm_buffer:
	.space	16
stack:	.space	256
stack_top:
