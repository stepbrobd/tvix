use std::cmp::Ordering;
use std::iter::{once, Chain, Once};
use std::ops::RangeInclusive;

use bstr::{BStr, ByteSlice, B};

/// Version strings can be broken up into Parts.
/// One Part represents either a string of digits or characters.
/// '.' and '_' represent deviders between parts and are not included in any part.
#[derive(PartialEq, Eq, Clone, Debug)]
pub enum VersionPart<'a> {
    Word(&'a BStr),
    Number(&'a BStr),
}

impl PartialOrd for VersionPart<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for VersionPart<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (VersionPart::Number(s1), VersionPart::Number(s2)) => {
                // Note: C++ Nix uses `int`, but probably doesn't make a difference
                // We trust that the splitting was done correctly and parsing will work
                let n1: u64 = s1.to_str_lossy().parse().unwrap();
                let n2: u64 = s2.to_str_lossy().parse().unwrap();
                n1.cmp(&n2)
            }

            // `pre` looses unless the other part is also a `pre`
            (VersionPart::Word(x), VersionPart::Word(y)) if *x == B("pre") && *y == B("pre") => {
                Ordering::Equal
            }
            (VersionPart::Word(x), _) if *x == B("pre") => Ordering::Less,
            (_, VersionPart::Word(y)) if *y == B("pre") => Ordering::Greater,

            // Number wins against Word
            (VersionPart::Number(_), VersionPart::Word(_)) => Ordering::Greater,
            (VersionPart::Word(_), VersionPart::Number(_)) => Ordering::Less,

            (VersionPart::Word(w1), VersionPart::Word(w2)) => w1.cmp(w2),
        }
    }
}

/// Type used to hold information about a VersionPart during creation
enum InternalPart {
    Number { range: RangeInclusive<usize> },
    Word { range: RangeInclusive<usize> },
    Break,
}

/// An iterator which yields the parts of a version string.
///
/// This can then be directly used to compare two versions
pub struct VersionPartsIter<'a> {
    cached_part: InternalPart,
    iter: bstr::CharIndices<'a>,
    version: &'a BStr,
}

impl<'a> VersionPartsIter<'a> {
    pub fn new(version: &'a BStr) -> Self {
        Self {
            cached_part: InternalPart::Break,
            iter: version.char_indices(),
            version,
        }
    }

    /// Create an iterator that yields all version parts followed by an additional
    /// `VersionPart::Word("")` part (i.e. you can think of this as
    /// `builtins.splitVersion version ++ [ "" ]`). This is necessary, because
    /// Nix's `compareVersions` is not entirely lexicographical: If we have two
    /// equal versions, but one is longer, the longer one is only considered
    /// greater if the first additional part of the longer version is not `pre`,
    /// e.g. `2.3 > 2.3pre`. It is otherwise lexicographical, so peculiar behavior
    /// like `2.3 < 2.3.0pre` ensues. Luckily for us, this means that we can
    /// lexicographically compare two version strings, _if_ we append an extra
    /// component to both versions.
    pub fn new_for_cmp(version: &'a BStr) -> Chain<Self, Once<VersionPart<'a>>> {
        Self::new(version).chain(once(VersionPart::Word("".into())))
    }
}

impl<'a> Iterator for VersionPartsIter<'a> {
    type Item = VersionPart<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let char = self.iter.next();

        if char.is_none() {
            let cached_part = std::mem::replace(&mut self.cached_part, InternalPart::Break);
            match cached_part {
                InternalPart::Break => return None,
                InternalPart::Number { range } => {
                    return Some(VersionPart::Number(&self.version[range]))
                }
                InternalPart::Word { range } => {
                    return Some(VersionPart::Word(&self.version[range]))
                }
            }
        }

        let (start, end, char) = char.unwrap();
        match char {
            // Divider encountered
            '.' | '-' => {
                let cached_part = std::mem::replace(&mut self.cached_part, InternalPart::Break);
                match cached_part {
                    InternalPart::Number { range } => {
                        Some(VersionPart::Number(&self.version[range]))
                    }
                    InternalPart::Word { range } => Some(VersionPart::Word(&self.version[range])),
                    InternalPart::Break => self.next(),
                }
            }

            // digit encountered
            _ if char.is_ascii_digit() => {
                let cached_part = std::mem::replace(
                    &mut self.cached_part,
                    InternalPart::Number {
                        range: start..=(end - 1),
                    },
                );
                match cached_part {
                    InternalPart::Number { range } => {
                        self.cached_part = InternalPart::Number {
                            range: *range.start()..=*range.end() + 1,
                        };
                        self.next()
                    }
                    InternalPart::Word { range } => Some(VersionPart::Word(&self.version[range])),
                    InternalPart::Break => self.next(),
                }
            }

            // char encountered
            _ => {
                let mut cached_part = InternalPart::Word {
                    range: start..=(end - 1),
                };
                std::mem::swap(&mut cached_part, &mut self.cached_part);
                match cached_part {
                    InternalPart::Word { range } => {
                        self.cached_part = InternalPart::Word {
                            range: *range.start()..=*range.end() + char.len_utf8(),
                        };
                        self.next()
                    }
                    InternalPart::Number { range } => {
                        Some(VersionPart::Number(&self.version[range]))
                    }
                    InternalPart::Break => self.next(),
                }
            }
        }
    }
}
