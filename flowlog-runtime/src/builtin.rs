//! Domains of the partial built-in functions, `to_number` and `substr`.
//! A partial function has no value outside its domain, so a rule has no
//! row for a binding that would evaluate one there; generated code tests
//! these before it evaluates the call and drops a row that fails.

/// Returns `true` if `to_number(text)` is defined: `text` is an `int32`
/// as Rust's `str::parse` reads one, an optional `+` or `-` then ASCII
/// digits, with nothing around them, in range.
#[must_use]
pub fn to_number_defined(text: &str) -> bool {
    text.parse::<i32>().is_ok()
}

/// Returns `true` if `substr(text, start, len)` is defined: `start` is a
/// character position from `0` to `text`'s length, both included, and
/// `len` is not negative. A `len` past the end of `text` is defined and
/// takes the rest, as Souffle's `substr` does; Souffle warns and answers
/// `""` for a `start` outside `text`, where this has no value.
#[must_use]
pub fn substr_defined(text: &str, start: i32, len: i32) -> bool {
    len >= 0 && usize::try_from(start).is_ok_and(|start| start <= text.chars().count())
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    // Cases: text, defined.
    #[rstest]
    #[case::digits("42", true)]
    #[case::signed("-42", true)]
    #[case::plus("+42", true)]
    #[case::leading_zeros("007", true)]
    #[case::min("-2147483648", true)]
    #[case::max("2147483647", true)]
    #[case::past_max("2147483648", false)]
    #[case::past_min("-2147483649", false)]
    #[case::empty("", false)]
    #[case::sign_alone("-", false)]
    #[case::leading_space(" 42", false)]
    #[case::trailing_space("42 ", false)]
    #[case::letters("x", false)]
    #[case::fraction("4.2", false)]
    fn to_number_is_defined_where_the_text_parses_as_an_int32(
        #[case] text: &str,
        #[case] defined: bool,
    ) {
        assert_eq!(to_number_defined(text), defined);
    }

    // Cases: text, start, len, defined.
    #[rstest]
    #[case::whole("abc", 0, 3, true)]
    #[case::past_the_end("abc", 1, 10, true)]
    #[case::empty_at_the_end("abc", 3, 0, true)]
    #[case::empty_text("", 0, 0, true)]
    #[case::start_past_the_end("abc", 4, 0, false)]
    #[case::negative_start("abc", -1, 2, false)]
    #[case::negative_len("abc", 0, -1, false)]
    // `\u{e9}` is two bytes: the end is the third character, not the
    // fourth byte.
    #[case::by_character("\u{e9}ab", 3, 0, true)]
    #[case::by_character_past_the_end("\u{e9}ab", 4, 0, false)]
    fn substr_is_defined_from_the_first_character_to_the_end(
        #[case] text: &str,
        #[case] start: i32,
        #[case] len: i32,
        #[case] defined: bool,
    ) {
        assert_eq!(substr_defined(text, start, len), defined);
    }
}
