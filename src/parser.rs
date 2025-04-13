use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case},
    character::complete::{digit1, space0},
    combinator::{map_res, opt, value},
    sequence::{separated_pair, tuple},
    IResult, Parser as _,
};

#[derive(Clone)]
pub enum CmpOperator {
    Equal,
    GreaterEqual,
    GreaterThan,
    LessEqual,
    LessThan,
    NotEqual,
}

impl From<&str> for CmpOperator {
    fn from(val: &str) -> Self {
        match val {
            ">" => Self::GreaterThan,
            ">=" => Self::GreaterEqual,
            "<" => Self::LessThan,
            "<=" => Self::LessEqual,
            "=" | "==" | "===" => Self::Equal,
            "!=" | "!==" => Self::NotEqual,
            _ => Self::Equal,
        }
    }
}
impl From<CmpOperator> for String {
    fn from(val: CmpOperator) -> Self {
        match val {
            CmpOperator::Equal => "==".to_owned(),
            CmpOperator::GreaterEqual => ">=".to_owned(),
            CmpOperator::GreaterThan => ">".to_owned(),
            CmpOperator::LessEqual => "<=".to_owned(),
            CmpOperator::LessThan => "<".to_owned(),
            CmpOperator::NotEqual => "!=".to_owned(),
        }
    }
}

impl From<&CmpOperator> for String {
    fn from(val: &CmpOperator) -> Self {
        match val {
            CmpOperator::Equal => "==".to_owned(),
            CmpOperator::GreaterEqual => ">=".to_owned(),
            CmpOperator::GreaterThan => ">".to_owned(),
            CmpOperator::LessEqual => "<=".to_owned(),
            CmpOperator::LessThan => "<".to_owned(),
            CmpOperator::NotEqual => "!=".to_owned(),
        }
    }
}

impl std::fmt::Display for CmpOperator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", String::from(self))
    }
}

pub struct Dice {
    pub num: u32,
    pub dice: u64,
    pub cmp: Option<(CmpOperator, u128)>,
}
pub const fn cmp_with_operator(operator: &CmpOperator, left: u128, right: u128) -> bool {
    match *operator {
        CmpOperator::Equal => left == right,
        CmpOperator::GreaterEqual => left >= right,
        CmpOperator::GreaterThan => left > right,
        CmpOperator::LessEqual => left <= right,
        CmpOperator::LessThan => left < right,
        CmpOperator::NotEqual => left != right,
    }
}

fn parse_cmp_operator(input: &str) -> IResult<&str, CmpOperator> {
    alt((
        value(CmpOperator::Equal, alt((tag("=="), tag("=")))),
        value(CmpOperator::GreaterEqual, tag(">=")),
        value(CmpOperator::GreaterThan, tag(">")),
        value(CmpOperator::LessEqual, tag("<=")),
        value(CmpOperator::LessThan, tag("<")),
        value(CmpOperator::NotEqual, tag("!=")),
    ))(input)
}

pub fn parse_dice(input: &str) -> IResult<&str, Dice> {
    tuple((
        separated_pair(
            map_res(digit1, str::parse),
            tag_no_case("d"),
            map_res(digit1, str::parse),
        ),
        opt(tuple((
            space0,
            parse_cmp_operator,
            space0,
            map_res(digit1, str::parse),
        ))),
    ))
    .map(|((num, dice), cmp)| Dice {
        num,
        dice,
        cmp: cmp.map(|(_, op, _, operand)| (op, operand)),
    })
    .parse(input)
}
