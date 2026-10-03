//! Slash-command tokeniser.
//!
//! Input format: `/<name> <arg> <arg2> ...`. Args may be quoted with `"`.
//! Backslash escapes `\"` and `\\` inside a quoted string. Unquoted args
//! split on whitespace.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedCommand {
    pub name: String,         // lower-cased; no leading `/`
    pub args: Vec<String>,    // case preserved
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    Empty,
    NotACommand,
    OnlySlash,
    UnclosedQuote,
    BadEscape,
}

pub fn parse(input: &str) -> Result<ParsedCommand, ParseError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(ParseError::Empty);
    }
    let body = trimmed.strip_prefix('/').ok_or(ParseError::NotACommand)?;
    if body.is_empty() {
        return Err(ParseError::OnlySlash);
    }

    let tokens = tokenise(body)?;
    let mut iter = tokens.into_iter();
    let name = iter.next().ok_or(ParseError::OnlySlash)?.to_lowercase();
    let args: Vec<String> = iter.collect();
    Ok(ParsedCommand { name, args })
}

fn tokenise(input: &str) -> Result<Vec<String>, ParseError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut escape = false;
    let mut had_token = false;

    for ch in input.chars() {
        if escape {
            match ch {
                '"' => current.push('"'),
                '\\' => current.push('\\'),
                'n' => current.push('\n'),
                't' => current.push('\t'),
                _ => return Err(ParseError::BadEscape),
            }
            escape = false;
            had_token = true;
            continue;
        }
        if ch == '\\' && in_quotes {
            escape = true;
            continue;
        }
        if ch == '"' {
            in_quotes = !in_quotes;
            had_token = true;
            continue;
        }
        if !in_quotes && ch.is_whitespace() {
            if had_token {
                tokens.push(std::mem::take(&mut current));
                had_token = false;
            }
            continue;
        }
        current.push(ch);
        had_token = true;
    }

    if in_quotes {
        return Err(ParseError::UnclosedQuote);
    }
    if escape {
        return Err(ParseError::BadEscape);
    }
    if had_token {
        tokens.push(current);
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_command() {
        assert_eq!(
            parse("/time set 6000").unwrap(),
            ParsedCommand {
                name: "time".to_string(),
                args: vec!["set".to_string(), "6000".to_string()],
            }
        );
    }

    #[test]
    fn lowercases_name_only() {
        let p = parse("/TIME GET").unwrap();
        assert_eq!(p.name, "time");
        assert_eq!(p.args, vec!["GET".to_string()]);
    }

    #[test]
    fn handles_quoted_args() {
        let p = parse("/say \"hello world\"").unwrap();
        assert_eq!(p.name, "say");
        assert_eq!(p.args, vec!["hello world".to_string()]);
    }

    #[test]
    fn handles_quoted_with_escapes() {
        let p = parse("/say \"she said \\\"hi\\\"\"").unwrap();
        assert_eq!(p.args, vec!["she said \"hi\"".to_string()]);
    }

    #[test]
    fn empty_input() {
        assert_eq!(parse("").unwrap_err(), ParseError::Empty);
        assert_eq!(parse("   ").unwrap_err(), ParseError::Empty);
    }

    #[test]
    fn not_a_command() {
        assert_eq!(parse("hello").unwrap_err(), ParseError::NotACommand);
    }

    #[test]
    fn only_slash() {
        assert_eq!(parse("/").unwrap_err(), ParseError::OnlySlash);
        assert_eq!(parse("/   ").unwrap_err(), ParseError::OnlySlash);
    }

    #[test]
    fn unclosed_quote() {
        assert_eq!(parse("/say \"oops").unwrap_err(), ParseError::UnclosedQuote);
    }

    #[test]
    fn bad_escape() {
        assert_eq!(
            parse("/say \"\\q\"").unwrap_err(),
            ParseError::BadEscape
        );
    }

    #[test]
    fn collapses_whitespace() {
        let p = parse("/help     time   ").unwrap();
        assert_eq!(p.name, "help");
        assert_eq!(p.args, vec!["time".to_string()]);
    }

    #[test]
    fn empty_quoted_arg_is_kept() {
        let p = parse("/say \"\"").unwrap();
        assert_eq!(p.args, vec!["".to_string()]);
    }

    #[test]
    fn no_args() {
        let p = parse("/help").unwrap();
        assert_eq!(p.name, "help");
        assert!(p.args.is_empty());
    }
}
