# Compatibility with asm68k

This describes the behaviour of asm68k (SN 68k version 2.53) that slop68k
reproduces. Where it is not in the manual, or not as the manual has it, it
was found by assembling test sources with asm68k itself; most of it is
covered by the cases in `tests/cases`.

## How a program is assembled

The source is read once. Every instruction is given its size when it is
read, from what is known at that point, and values that depend on symbols
defined further on are filled in at the end. It follows that:

- A reference to a symbol that is not yet defined gets the longest form:
  absolute addresses are long, branches without a size are word sized.
- The optimisation options (`ow`, `os`, `op`, `oz`, `oaq`, `osq`, `omq`)
  have no effect on references to symbols defined further on.
- `SET`, `IF`, `REPT`, `DS`, `DCB`, `ORG`, `RS` and the like need operands
  whose value is known when they are read.
- The listing shows code as it was when it was read, with zeros for values
  not known at the time.

If there are errors, the values left to fill in are not looked at, so no
errors are reported for them ("Errors during pass 1 - pass 2 aborted").

## Statements

- Anything in the first column is a label. Elsewhere a label has to end
  with a colon.
- The operand field ends at the first white space that does not follow a
  comma; what comes after is a comment. `dc.b 1, 2` defines two bytes and
  `dc.b 1 , 2` one. With `opt ws+` white space is allowed throughout and
  comments have to start with a semicolon.
- A line starting with `*` or `;` is a comment.
- A line ending with `&` is continued on the next, unless the `&` is part
  of a comment.
- Names are not case sensitive unless `opt c+` is given. Dots are part of
  names: in an expression `label.w` is a name of its own, while as an
  operand it is `label` addressed as absolute short.
- A label on a line with an instruction at an odd address is given the
  address the instruction is moved to.

## Expressions

Values are 32 bits wide. Precedence, from highest to lowest:

| Operators             |                                   |
|-----------------------|-----------------------------------|
| `+` `-` `~` `!`       | unary                             |
| `<<` `>>`             | shifts                            |
| `&` `!` `\|` `^`      | and, or, or, exclusive or         |
| `*` `/` `%`           | multiply, divide, remainder       |
| `+` `-`               | add, subtract                     |
| `<` `>` `<=` `>=`     | comparisons                       |
| `=` `<>`              | equal, not equal                  |

This is not the order of C: `1+2&3` is `1+(2&3)` and `2*6&3` is `2*(6&3)`.
Operators of the same precedence are applied from left to right.

- Comparisons are signed and give -1 when true.
- `>>` shifts in zeros. Shift counts are taken modulo 32.
- Division rounds towards zero.
- Numbers: decimal, `$ff`, `0xff`, `%1010`, and with `opt an+` also
  `0ffh`, `1010b` and `99d`. A leading zero does not make a number octal.
- Character constants hold up to four characters: `'AB'` is `$4142`.
  Quotes are doubled to get them into a string of their own kind.
- `*` is the address of the start of the statement, `@` on its own the
  address at which the value being defined will be stored.
- Where a macro parameter that was not given leaves an operator with
  nothing in front of it, the missing operand is zero.

## Instructions

- `add`, `sub`, `and`, `or`, `eor` and `cmp` with an immediate operand are
  assembled as `addi`, `subi` and so on, also when the other operand is a
  data register.
- With an address register as destination they, and `addi`, `subi` and
  `cmpi`, are assembled as `adda`, `suba` and `cmpa`.
- `cmp (a0)+,(a1)+` is assembled as `cmpm`.
- A branch with the size `.s` or `.b` is short. A short branch to the next
  instruction is an error.
- `(pc)` without a displacement is the same as `0(pc)`.
- Shifts of a data register without a count shift by one.
- Bit numbers of `btst` and the like are not checked.

Values have to fit in their place, counting both as signed or unsigned:

| Place                                   | Range                       |
|-----------------------------------------|-----------------------------|
| Byte: `dc.b`, `move.b #n`               | -128 to 255                 |
| Word: `dc.w`, `move.w #n`               | -32768 to 65535             |
| `moveq`                                 | -128 to 255, with a warning above 127 |
| Displacement `d(An)`, absolute short    | -32768 to 32767             |
| Displacement `d(An,Xn)`, short branch   | -128 to 127                 |
| `addq`, `subq`, shift counts            | 1 to 8                      |

## Macros

- Arguments are separated by commas, also where these are in parentheses:
  `mac 4(a0,d0.w),d1` has three arguments. Commas in quotes and between
  `<` and `>` do not separate arguments.
- White space that does not follow a comma ends the arguments.
- An argument in quotes keeps its quotes.
- `\1` to `\31` and `\name` are replaced anywhere on a line, in strings
  too. A backslash after the name ends it: `\name\_end`.
- Named parameters and string equates are also replaced where their bare
  name is used in an operand.
- `\@` is an underscore and the number of the macro invocation, counted
  from the start of the program. Outside of macros it stays as it is.
- `\#name` and `\$name` are the value of a symbol in decimal and in
  hexadecimal, the latter in lower case.
- Macros are looked up after instructions and directives, so they cannot
  take the name of either.
- A macro may have a local name, such as `@helper`. It is then known
  within the scope of the labels around it.
- The lines of macros and loops are kept without comments and empty lines,
  and with what ends the operand field decided by the options that apply
  where the macro is defined.

## Local labels

A local label belongs to the code between two labels that are not local.
Any such label starts a new scope, including those defined in macros.
`EQU`, `SET` and `RS` do not, unless `opt d+` is given. Inside `MODULE` and
`MODEND` labels do not start a scope.

## Output

- A program placed with `ORG` is written from its lowest address to its
  highest. Gaps are filled with zeros, and where code overlaps what came
  last remains.
- `OBJ` blocks can be nested; `OBJEND` returns to the block around it.
- Sections of a program that is not linked follow each other in the order
  of their groups, starting at the address of the `ORG`.
- On entering a section, code is aligned to its alignment (word, by
  default).

## The listing

- Each line has the address, up to ten bytes of the code, and the source
  text from column 36. A `+` follows the code if there is more of it.
- Lines that define a symbol show its value as `=xxxxxxxx`.
- `IF`, `ELSE` and `ENDIF` are not listed, nor are lines that are skipped.
- A line that invokes a macro or ends a loop shows the code of the
  expansion. It is not listed at all if the expansion has an `IF` in it.
- With `/m` the lines of expansions are listed themselves, marked `M`.
- Without `/m` the last line of an included file is listed twice.
- `INCBIN` lines, and `DCB` and `DS` of more than 256 bytes, show no code.
- Empty lines end with a line feed only, the others with a carriage return
  and a line feed.

## Differences

- Without `/p`, asm68k writes a CPE executable. slop68k writes a binary
  file whether or not `/p` is given.
- `/l` writes an ELF object file, not a Psy-Q link file.
- Symbols in the symbol file are in the order of their addresses; asm68k
  writes them in the order of its hash table.
- `move usp,a0` is accepted without a size.
- With `radix 16`, asm68k gets numbers of more than one digit wrong. They
  are right here.
- asm68k knows some addressing modes of the 68020. They are errors here.
- asm68k expands macros defined with `MACROS` where it finds them in the
  definition of another macro. Here they are expanded along with the macro
  they are in, which shows in the listing only.
- With `/m`, asm68k lists an empty macro as a line with a single space.
- `REGS`, `UNIT` and `WORKSPACE`, which concern downloading to a target,
  are ignored.
- The date and time constants are in UTC. asm68k has them in local time,
  which `--local-time` asks for.
- Messages name files as they were given, not by their full path in upper
  case.
