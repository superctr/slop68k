//! slop68k - a 68000 macro assembler compatible with asm68k.
//!
//! The assembler reads source written for the SN Systems / Psy-Q `asm68k`
//! assembler and produces the same code. See [`asm::assemble`] for the
//! entry point and [`cli`] for the command line.

pub mod asm;
pub mod cli;
pub mod diag;
pub mod expr;
pub mod m68k;
pub mod options;
pub mod output;
pub mod source;
pub mod symbol;
pub mod timestamp;
pub mod value;

pub use asm::{assemble, Assembly, Config};
