//! A small terminal model for judging rendered prompt output.
//!
//! Written without the encoders' code, so it is an independent oracle:
//! [`interpret`] runs output containing SGR sequences and draws every visible
//! character with the [`Look`] it ends up with. Included per test crate with
//! `#[path = "common/sgr.rs"] mod sgr;`.
#![allow(dead_code)]

/// A color as a terminal would hold it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shade {
    /// One of the 16 basic colors: index 0-7, bright or not.
    Basic { index: u8, bright: bool },
    /// A 256-color palette index.
    Indexed(u16),
    /// A 24-bit color.
    Rgb(u16, u16, u16),
}

pub const BOLD: u8 = 0b0000_0001;
pub const DIM: u8 = 0b0000_0010;
pub const ITALIC: u8 = 0b0000_0100;
pub const UNDERLINE: u8 = 0b0000_1000;
pub const BLINK: u8 = 0b0001_0000;
pub const INVERSE: u8 = 0b0010_0000;
pub const HIDDEN: u8 = 0b0100_0000;
pub const STRIKE: u8 = 0b1000_0000;

/// What a character looks like once drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Look {
    pub attrs: u8,
    pub fg: Option<Shade>,
    pub bg: Option<Shade>,
}

/// `index` within the basic range starting at `base`, if `code` is in it.
fn basic(code: u16, base: u16, bright: bool) -> Option<Shade> {
    let index = u8::try_from(code.checked_sub(base)?).ok()?;
    (index < 8).then_some(Shade::Basic { index, bright })
}

/// Read the operands of `38`/`48` (`5;n` or `2;r;g;b`).
fn extended(codes: &mut impl Iterator<Item = u16>) -> Result<Shade, String> {
    match codes.next() {
        Some(5) => codes
            .next()
            .map(Shade::Indexed)
            .ok_or_else(|| "truncated 256-color operand".to_owned()),
        Some(2) => match (codes.next(), codes.next(), codes.next()) {
            (Some(r), Some(g), Some(b)) => Ok(Shade::Rgb(r, g, b)),
            _ => Err("truncated rgb operand".to_owned()),
        },
        other => Err(format!("unsupported extended-color selector {other:?}")),
    }
}

/// Apply one `ESC [ params m` sequence to `look`.
fn apply(look: &mut Look, params: &str) -> Result<(), String> {
    if params.is_empty() {
        *look = Look::default();
        return Ok(());
    }
    let numbers = params
        .split(';')
        .map(|part| {
            if part.is_empty() {
                Ok(0)
            } else {
                part.parse::<u16>()
                    .map_err(|err| format!("bad SGR parameter {part:?}: {err}"))
            }
        })
        .collect::<Result<Vec<u16>, String>>()?;
    let mut codes = numbers.into_iter();
    while let Some(code) = codes.next() {
        match code {
            0 => *look = Look::default(),
            1 => look.attrs |= BOLD,
            2 => look.attrs |= DIM,
            3 => look.attrs |= ITALIC,
            4 => look.attrs |= UNDERLINE,
            5 => look.attrs |= BLINK,
            7 => look.attrs |= INVERSE,
            8 => look.attrs |= HIDDEN,
            9 => look.attrs |= STRIKE,
            22 => look.attrs &= !(BOLD | DIM),
            23 => look.attrs &= !ITALIC,
            24 => look.attrs &= !UNDERLINE,
            25 => look.attrs &= !BLINK,
            27 => look.attrs &= !INVERSE,
            28 => look.attrs &= !HIDDEN,
            29 => look.attrs &= !STRIKE,
            30..=37 => look.fg = basic(code, 30, false),
            38 => look.fg = Some(extended(&mut codes)?),
            39 => look.fg = None,
            40..=47 => look.bg = basic(code, 40, false),
            48 => look.bg = Some(extended(&mut codes)?),
            49 => look.bg = None,
            90..=97 => look.fg = basic(code, 90, true),
            100..=107 => look.bg = basic(code, 100, true),
            other => return Err(format!("SGR code {other} is outside the oracle's model")),
        }
    }
    Ok(())
}

/// Each drawn character with the look it carries, plus the look left active.
pub type Drawing = (Vec<(char, Look)>, Look);

/// Run `output` through the terminal model: each drawn character with the
/// look it carries, plus the look left active at the end. Anything that is
/// not a plain character or an SGR sequence is an error.
pub fn interpret(output: &str) -> Result<Drawing, String> {
    let mut look = Look::default();
    let mut drawn = Vec::new();
    let mut chars = output.chars();
    while let Some(ch) = chars.next() {
        if ch != '\x1b' {
            drawn.push((ch, look));
            continue;
        }
        if chars.next() != Some('[') {
            return Err("escape that is not a CSI sequence".to_owned());
        }
        let mut params = String::new();
        loop {
            match chars.next() {
                Some('m') => break,
                Some(next) if next.is_ascii_digit() || next == ';' => params.push(next),
                other => {
                    return Err(format!(
                        "CSI {params:?} ends in {other:?}, not the SGR final byte"
                    ));
                }
            }
        }
        apply(&mut look, &params)?;
    }
    Ok((drawn, look))
}
