use std::{
    collections::HashSet,
    sync::{Mutex, MutexGuard},
};

// use once_cell::sync::Lazy;
use regex::Regex;
// static abbrs: Lazy<Mutex<AbbreviationSet>> = Lazy::new(|| Mutex::new(Default::default()));

// pub fn with_abbrs<R>(cb: impl FnOnce(&AbbreviationSet) -> R) -> R {
//     let abbrs_g = abbrs.lock().unwrap();
//     cb(&abbrs_g)
// }
//
// pub fn with_abbrs_mut<R>(cb: impl FnOnce(&mut AbbreviationSet) -> R) -> R {
//     let mut abbrs_g = abbrs.lock().unwrap();
//     cb(&mut abbrs_g)
// }

/// Controls where in the command line abbreviations may expand.
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Position {
    Command,  // expand in command position
    Anywhere, // expand in any token
}

#[derive(Debug)]
enum MatchType {
    /// The key (recognized token) - either a literal or a regex pattern.
    Literal(String),

    /// If set, use this regex to recognize tokens.
    /// If unset, the key is to be interpreted literally.
    /// Note that the fish interface enforces that regexes match the entire token;
    /// we accomplish this by surrounding the regex in ^ and $.
    Regex(Regex),
}

#[derive(Debug)]
pub struct Abbreviation {
    // Abbreviation name. This is unique within the abbreviation set.
    // This is used as the token to match unless we have a regex.
    pub name: String,

    pub key: MatchType,

    /// Replacement string.
    pub replacement: String,

    /// If set, the replacement is a function name.
    pub replacement_is_function: bool,

    /// Expansion position.
    pub position: Position,

    /// If set, then move the cursor to the first instance of this string in the expansion.
    pub set_cursor_marker: Option<String>,

    /// Mark if we came from a universal variable.
    pub from_universal: bool,
}

impl Abbreviation {
    // Construct from a name, a key which matches a token, a replacement token, a position, and
    // whether we are derived from a universal variable.
    pub fn new(
        name: String,
        key: MatchType,
        replacement: String,
        position: Position,
        from_universal: bool,
    ) -> Self {
        Self {
            name,
            key,
            replacement,
            replacement_is_function: false,
            position,
            set_cursor_marker: None,
            from_universal,
        }
    }

    // \return true if this is a regex abbreviation.
    pub fn is_regex(&self) -> bool {
        match self.key {
            MatchType::Regex(_) => true,
            _ => false,
        }
    }

    // \return true if we match a token at a given position.
    pub fn matches(&self, token: &str, position: Position) -> bool {
        if !self.matches_position(position) {
            return false;
        }
        match &self.key {
            MatchType::Literal(k) => k == token,
            MatchType::Regex(r) => r.captures(token).is_some(),
        }
    }

    // \return if we expand in a given position.
    fn matches_position(&self, position: Position) -> bool {
        return self.position == Position::Anywhere || self.position == position;
    }
}

/// The result of an abbreviation expansion.
pub struct Replacer {
    /// The string to use to replace the incoming token, either literal or as a function name.
    replacement: String,

    /// If true, treat 'replacement' as the name of a function.
    is_function: bool,

    /// If set, the cursor should be moved to the first instance of this string in the expansion.
    set_cursor_marker: Option<String>,
}

struct Replacement {
    /// The original range of the token in the command line.
    range: SourceRange,

    /// The string to replace with.
    text: String,

    /// The new cursor location, or none to use the default.
    /// This is relative to the original range.
    cursor: Option<usize>,
}

impl Replacement {
    /// Construct a replacement from a replacer.
    /// The \p range is the range of the text matched by the replacer in the command line.
    /// The text is passed in separately as it may be the output of the replacer's function.
    fn from(range: SourceRange, mut text: WString, set_cursor_marker: Option<WString>) -> Self {
        let mut cursor = None;
        if let Some(set_cursor_marker) = set_cursor_marker {
            let matched = text
                .as_char_slice()
                .windows(set_cursor_marker.len())
                .position(|w| w == set_cursor_marker.as_char_slice());

            if let Some(start) = matched {
                text.replace_range(start..(start + set_cursor_marker.len()), L!(""));
                cursor = Some(start + range.start as usize)
            }
        }
        Self {
            range,
            text,
            cursor,
        }
    }
}

#[derive(Default)]
pub struct AbbreviationSet {
    /// List of abbreviations, in definition order.
    abbrs: Vec<Abbreviation>,

    /// Set of used abbrevation names.
    /// This is to avoid a linear scan when adding new abbreviations.
    used_names: HashSet<String>,
}

impl AbbreviationSet {
    /// \return the list of replacers for an input token, in priority order.
    /// The \p position is given to describe where the token was found.
    pub fn r#match(&self, token: &str, position: Position) -> Vec<Replacer> {
        let mut result = vec![];

        // Later abbreviations take precedence so walk backwards.
        for abbr in self.abbrs.iter().rev() {
            if abbr.matches(token, position) {
                result.push(Replacer {
                    replacement: abbr.replacement.clone(),
                    is_function: abbr.replacement_is_function,
                    set_cursor_marker: abbr.set_cursor_marker.clone(),
                });
            }
        }
        return result;
    }

    /// \return whether we would have at least one replacer for a given token.
    pub fn has_match(&self, token: &str, position: Position) -> bool {
        self.abbrs.iter().any(|abbr| abbr.matches(token, position))
    }

    /// Add an abbreviation. Any abbreviation with the same name is replaced.
    pub fn add(&mut self, abbr: Abbreviation) {
        assert!(!abbr.name.is_empty(), "Invalid name");
        let inserted = self.used_names.insert(abbr.name.clone());
        if !inserted {
            // Name was already used, do a linear scan to find it.
            let index = self
                .abbrs
                .iter()
                .position(|a| a.name == abbr.name)
                .expect("Abbreviation not found though its name was present");

            self.abbrs.remove(index);
        }
        self.abbrs.push(abbr);
    }

    /// Rename an abbreviation. This asserts that the old name is used, and the new name is not; the
    /// caller should check these beforehand with has_name().
    pub fn rename(&mut self, old_name: &wstr, new_name: &wstr) {
        let erased = self.used_names.remove(old_name);
        let inserted = self.used_names.insert(new_name.to_owned());
        assert!(
            erased && inserted,
            "Old name not found or new name already present"
        );
        for abbr in self.abbrs.iter_mut() {
            if abbr.name == old_name {
                abbr.name = new_name.to_owned();
                break;
            }
        }
    }

    /// Erase an abbreviation by name.
    /// \return true if erased, false if not found.
    pub fn erase(&mut self, name: &str) -> bool {
        let erased = self.used_names.remove(name);
        if !erased {
            return false;
        }
        for (index, abbr) in self.abbrs.iter().enumerate().rev() {
            if abbr.name == name {
                self.abbrs.remove(index);
                return true;
            }
        }
        panic!("Unable to find named abbreviation");
    }

    /// \return true if we have an abbreviation with the given name.
    pub fn has_name(&self, name: &str) -> bool {
        self.used_names.contains(name)
    }

    /// \return a reference to the abbreviation list.
    pub fn list(&self) -> &[Abbreviation] {
        &self.abbrs
    }
}

pub struct GlobalAbbrs<'a> {
    g: MutexGuard<'a, AbbreviationSet>,
}

impl<'a> GlobalAbbrs<'a> {
    fn add(
        &mut self,
        name: String,
        key: String,
        replacement: String,
        position: Position,
        from_universal: bool,
    ) {
        self.g.add(Abbreviation::new(
            name,
            key,
            replacement,
            position,
            from_universal,
        ));
    }

    fn erase(&mut self, name: &str) {
        self.g.erase(name);
    }
}
