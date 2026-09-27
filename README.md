# slop68k

A 68000 macro assembler that takes the place of `asm68k`, the assembler of
the SN Systems / Psy-Q development system. It reads the same source, takes
the same command line and produces the same output, and it runs anywhere
Rust does.

Output is compared byte for byte with that of asm68k 2.53, for object code
and listings alike. See [Testing](#testing).

## Building

```
cargo build --release
```

The program is `target/release/slop68k`. It depends on nothing but the Rust
standard library.

## Use

There are two ways of writing the command line, which can be mixed.

### As asm68k

```
slop68k /k /p /o ae- source,object,symbols,listing
```

This is the command line of asm68k, so in a build script written for it, it
is enough to change the name of the program. File names may use
backslashes, and are found whatever their case, as sources written on
Windows expect.

| Switch       |                                                         |
|--------------|---------------------------------------------------------|
| `/p`         | Produce a pure binary file.                             |
| `/ps`        | Produce Motorola S-records.                             |
| `/l`         | Produce a relocatable object file (ELF).                |
| `/o options` | Set assembler options, as with `OPT`: `/o ae-,ow+`      |
| `/e n=x`     | Define the symbol `n` with the value `x`.               |
| `/j path`    | Add a directory to search for `INCLUDE` and `INCBIN`.   |
| `/m`         | List the lines of macro expansions.                     |
| `/c`         | List code skipped by conditional assembly.              |
| `/k`         | Accept `IFEQ`, `IFND` and the like. Always on.          |

### As other tools

```
slop68k -I include -D DEBUG=1 -O ae-,c+ -o build/main.o src/main.68k
```

| Option                 |                                                   |
|------------------------|---------------------------------------------------|
| `-o`, `--output FILE`  | Write the object code to `FILE`.                  |
| `-f`, `--format FMT`   | Format of the object code: `bin`, `srec`, `elf`.  |
| `-l`, `--listing FILE` | Write a listing.                                  |
| `-s`, `--symbols FILE` | Write a symbol file.                              |
| `-I`, `--include DIR`  | Add a directory to search for included files.     |
| `-D`, `--define N[=X]` | Define the symbol `N` with the value `X`, or 1.   |
| `-O`, `--options LIST` | Set assembler options, as with `OPT`.             |
| `-m`, `--list-macros`  | List the lines of macro expansions.               |
| `--list-skipped`       | List code skipped by conditional assembly.        |
| `--local-time`         | Use local time, not UTC, for the date and time.   |
| `-q`, `--quiet`        | Print nothing but warnings and errors.            |

Options may come before or after the source, and a value may be attached
to its option, as in `-Iinclude` and `--output=main.o`. Without `-f`, `/p`,
`/ps` or `/l` the name of the output decides its format: ELF for `.o`,
S-records for `.s19`, `.s28`, `.s37` and `.srec`, and binary for anything
else. Without an output file the source is assembled and nothing written.

### Messages

Messages go to standard output, in the format of asm68k:

```
src/main.68k(12) : Error : Illegal addressing mode
	move.b	d0,a0
```

The exit status is zero if there were no errors.

### Time of assembly

`_year`, `_month` and the other date and time constants are taken from the
clock, in UTC. With `--local-time` they are in local time, as they are
with asm68k. This is there on Linux only; elsewhere it gives a warning and
UTC.

For builds that have to come out the same every time, set
`SOURCE_DATE_EPOCH` to the time to use, in seconds since 1970.

### Object files

Object files are in the ELF format, for the GNU linker, to be linked with
code from GCC. In a makefile:

```make
%.o: %.68k
	slop68k -q -O c+ -I include -o $@ $<
```

- Each `SECTION` becomes a section of the object file. Code that is not in
  a section goes in `.text`. Name sections `.text`, `.data`, `.rodata` and
  `.bss` to have them treated as the compiler's are.
- `XDEF`, `GLOBAL` and `PUBLIC` make symbols global; `XREF` and `GLOBAL`
  declare those defined elsewhere. A symbol that is neither defined nor
  declared is an error, as it is with asm68k.
- `ORG` cannot be used.
- Names are not case sensitive unless `c+` is among the options. They are
  written to the object file as they are spelled where they are defined or
  declared.
- GCC for `m68k-elf` does not put an underscore in front of the names of C,
  so a function is called by its name: `jsr main`.
- Code is for the 68000, so C is to be compiled with `-m68000`.

## Compatibility

[docs/compatibility.md](docs/compatibility.md) describes the behaviour of
asm68k that this assembler reproduces, much of which is not in the manual,
and the ways in which the two differ. The main differences:

- Without `/p`, asm68k writes an executable in its own CPE format. Here the
  default is a binary file.
- `/l` writes ELF rather than Psy-Q link files.
- Options that start with a dash are an addition.
- The order of symbols in the symbol file differs.
- Downloading to a target machine is not there.

## Testing

```
cargo test
```

The cases in `tests/cases` are assembled and the object code, listing and
messages compared with those of asm68k, which are kept alongside them. To
make them anew, with Wine and a copy of `asm68k.exe`:

```
ASM68K=/path/to/asm68k.exe tests/regen.sh
```

If there is a GCC toolchain for `m68k-elf`, programs written for this
assembler and for the GNU assembler are linked with C code, and the results
compared. The toolchain is looked for in the `PATH` and in
`/opt/toolchains/m68k-elf/bin`; set `M68K_ELF` to the prefix of its
programs (such as `/usr/local/bin/m68k-elf-`) if it is somewhere else.
Without a toolchain these tests do nothing.

Beyond these the assembler has been checked against asm68k with several
complete programs, among them the [Sonic the Hedgehog disassembly][s1] in
its version for asm68k: 153,000 lines that assemble to the same 512 KB.

## License

This is free to use for any purpose, under the terms of either the
[BSD Zero Clause License](LICENSE-0BSD) or the [Unlicense](UNLICENSE), as
you prefer.

[s1]: https://github.com/sonicretro/s1disasm/tree/asm68k
