	.globl	short_refs
short_refs:
	bsr.s	near_c
	bsr.w	c_function
	move.w	c_function(%pc),%d0
	move.w	c_function(%pc,%d0.w),%d1
	lea	asm_table(%pc),%a0
	move.w	(asm_table).w,%d0
	move.w	(asm_table+2).w,%d0
	move.b	#small_const,%d0
	move.w	#small_const+1,%d0
	move.l	#small_const,%d0
	move.w	small_const(%a0),%d0
	move.b	small_const(%a0,%d1.l),%d0
	cmpi.w	#small_const,c_value
	.byte	small_const,small_const+2
	.word	small_const
	beq.s	near_c
	bne.w	c_function
	rts
near_c:	jmp	c_function
