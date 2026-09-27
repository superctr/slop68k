OUTPUT_FORMAT("elf32-m68k")
ENTRY(_start)
MEMORY
{
    rom (rx)  : ORIGIN = 0x00000200, LENGTH = 0x10000
    ram (rwx) : ORIGIN = 0x00ff0000, LENGTH = 0x10000
}
SECTIONS
{
    .text   : { *(.text .text.*) . = ALIGN(2); } > rom
    .rodata : { *(.rodata .rodata.*) . = ALIGN(2); } > rom
    .data   : { *(.data .data.*) . = ALIGN(2); } > ram AT > rom
    .bss    : { *(.bss .bss.*) *(COMMON) . = ALIGN(2); } > ram
    /DISCARD/ : { *(.comment) *(.note*) }
}
