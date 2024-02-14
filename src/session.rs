use winnow::{
    ascii::{alphanumeric1, line_ending, multispace1, space0},
    combinator::{alt, delimited, empty, opt, repeat, separated},
    error::StrContext,
    prelude::*,
    token::literal,
};


#[derive(Debug)]
pub struct ZellijSession {
    pub name: String,
    pub exited: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Status {
    Exited,
    Else,
}

pub fn parse_zellij_ls(input: &mut &str) -> PResult<Vec<ZellijSession>> {
    fn session_name(input: &mut &str) -> PResult<String> {
        repeat(1.., alt((alphanumeric1, literal("-"), literal("_"))))
            .map(|text: Vec<&str>| text.join(""))
            .context(StrContext::Label("session name"))
            .parse_next(input)
    }

    fn status(input: &mut &str) -> PResult<Status> {
        alt((
            "EXITED - attach to resurrect".map(|_| Status::Exited),
            "current".map(|_| Status::Else),
            empty.value(Status::Else),
        ))
        .context(StrContext::Label("session status"))
        .parse_next(input)
    }

    (
        separated(
            ..,
            (
                session_name,
                ' ',
                delimited::<_, _, String, _, _, _, _, _>(
                    '[',
                    repeat(.., alt((alphanumeric1, multispace1))),
                    ']',
                ),
                space0,
                opt(delimited('(', status, ')')),
            )
                .map(|(name, _, _, _, opt_status)| ZellijSession {
                    name,
                    exited: opt_status
                        .map(|status| status == Status::Exited)
                        .unwrap_or(false),
                })
                .context(StrContext::Label("session entry")),
            line_ending,
        ),
        opt(line_ending),
    )
        .map(|res| res.0)
        .context(StrContext::Label("session list"))
        .parse_next(input)
}
