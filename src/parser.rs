use std::fmt;
use std::ops::Add;

use chrono::{DateTime, Datelike, Duration, NaiveDateTime, Timelike};
use chrono_tz::Tz;
use pest::error::ErrorVariant;
use pest::iterators::Pair;
use pest::Parser;
use pest_derive::Parser;

use crate::location::find_zone;

#[derive(Debug)]
pub enum DateParseError {
    Parser(pest::error::Error<Rule>),
    Garbage(String),
}

impl std::error::Error for DateParseError {}

impl fmt::Display for DateParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        struct Enumerate<'a, T: fmt::Debug>(&'a [T]);

        impl<'a, T: fmt::Debug> fmt::Display for Enumerate<'a, T> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                for (idx, item) in self.0.iter().enumerate() {
                    if idx > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{:?}", item)?;
                }
                Ok(())
            }
        }

        match self {
            DateParseError::Parser(p) => {
                write!(f, "invalid syntax (")?;
                match &p.variant {
                    ErrorVariant::ParsingError {
                        positives,
                        negatives,
                    } => match (negatives.is_empty(), positives.is_empty()) {
                        (false, false) => write!(
                            f,
                            "unexpected {}; expected {}",
                            Enumerate(&negatives),
                            Enumerate(&positives)
                        )?,
                        (false, true) => write!(f, "unexpected {}", Enumerate(&negatives))?,
                        (true, false) => write!(f, "expected {}", Enumerate(&positives))?,
                        (true, true) => write!(f, "unknown parsing error")?,
                    },
                    ErrorVariant::CustomError { message } => write!(f, "{}", message)?,
                }
                write!(f, ")")?;
                Ok(())
            }
            DateParseError::Garbage(leftover) => {
                write!(f, "invalid syntax (unsure how to interpret {:?})", leftover)
            }
        }
    }
}

#[derive(Parser)]
#[grammar = "date_grammar.pest"]
pub struct DateParser;

/// Represents a human readable date expression
#[derive(Debug)]
pub struct Expr<'a> {
    time_spec: Option<TimeSpec>,
    date_spec: Option<DateSpec>,
    locations: Vec<&'a str>,
}

impl<'a> Expr<'a> {
    /// Parses an expression from a string.
    pub fn parse(value: &'a str) -> Result<Expr<'a>, DateParseError> {
        parse_input(value)
    }

    /// Returns the location if available.
    pub fn location(&self) -> Option<&str> {
        self.locations.get(0).copied()
    }

    /// Returns the target locations if available.
    pub fn to_locations(&self) -> &[&str] {
        self.locations.get(1..).unwrap_or_default()
    }

    /// Applies the expression to a current reference date.
    pub fn apply(&self, mut date: DateTime<Tz>) -> DateTime<Tz> {
        match self.time_spec {
            Some(TimeSpec::Abs {
                hour,
                minute,
                second,
            }) => {
                date = date
                    .with_hour(hour as u32)
                    .unwrap()
                    .with_minute(minute as u32)
                    .unwrap()
                    .with_second(second as u32)
                    .unwrap();
            }
            Some(TimeSpec::Rel {
                hours,
                minutes,
                seconds,
            }) => {
                date = date.add(Duration::hours(hours as i64));
                date = date.add(Duration::minutes(minutes as i64));
                date = date.add(Duration::seconds(seconds as i64));
            }
            None => {}
        }
        match self.date_spec {
            Some(DateSpec::Abs { day, month, year }) => {
                date = date.with_day(day as u32).unwrap();
                if let Some(month) = month {
                    date = date.with_month(month as u32).unwrap();
                }
                if let Some(year) = year {
                    date = date.with_year(year).unwrap();
                }
            }
            Some(DateSpec::Rel { days }) => {
                date = date.add(Duration::days(days as i64));
            }
            None => {}
        }
        date
    }
}

#[derive(Debug)]
pub enum TimeSpec {
    Abs {
        hour: i32,
        minute: i32,
        second: i32,
    },
    Rel {
        hours: i32,
        minutes: i32,
        seconds: i32,
    },
}

#[derive(Debug)]
pub enum DateSpec {
    Abs {
        day: i32,
        month: Option<i32>,
        year: Option<i32>,
    },
    Rel {
        days: i32,
    },
}

fn as_int(pair: Pair<Rule>) -> i32 {
    pair.into_inner().next().unwrap().as_str().parse().unwrap()
}

fn parse_input(expr: &str) -> Result<Expr<'_>, DateParseError> {
    let expr = expr.trim();
    let pair = DateParser::parse(Rule::spec, expr)
        .map_err(DateParseError::Parser)?
        .next()
        .unwrap();

    if pair.as_str() != expr {
        return Err(DateParseError::Garbage(
            expr[pair.as_str().len()..].to_string(),
        ));
    }

    let mut rv = Expr {
        time_spec: None,
        date_spec: None,
        locations: vec![],
    };
    let mut unix_time = false;

    for piece in pair.into_inner() {
        match piece.as_rule() {
            Rule::location => {
                for loc in piece.as_str().split("->") {
                    let loc = loc.trim();
                    if !loc.is_empty() {
                        rv.locations.push(loc);
                    }
                }
            }
            Rule::unix_time => {
                let ts: i64 = piece.into_inner().next().unwrap().as_str().parse().unwrap();
                let dt = NaiveDateTime::from_timestamp(ts, 0);
                rv.time_spec = Some(TimeSpec::Abs {
                    hour: dt.hour() as _,
                    minute: dt.minute() as _,
                    second: dt.second() as _,
                });
                rv.date_spec = Some(DateSpec::Abs {
                    day: dt.day() as _,
                    month: Some(dt.month() as _),
                    year: Some(dt.year() as _),
                });
                unix_time = true;
            }
            Rule::abs_time => {
                let mut now = false;
                for abs_time_piece in piece.into_inner() {
                    match abs_time_piece.as_rule() {
                        Rule::time => {
                            let mut hour = 0;
                            let mut minute = 0;
                            let mut second = 0;
                            for time_piece in abs_time_piece.into_inner() {
                                match time_piece.as_rule() {
                                    Rule::HH12 | Rule::HH24 => {
                                        hour = time_piece.as_str().parse::<i32>().unwrap();
                                    }
                                    Rule::MM => {
                                        minute = time_piece.as_str().parse::<i32>().unwrap();
                                    }
                                    Rule::SS => {
                                        second = time_piece.as_str().parse::<i32>().unwrap();
                                    }
                                    Rule::meridiem => {
                                        if matches!(
                                            time_piece.into_inner().next().unwrap().as_rule(),
                                            Rule::pm
                                        ) {
                                            // don't change for 12pm
                                            if hour != 12 {
                                                hour += 12;
                                            }
                                        } else {
                                            // special case 12am = midnight
                                            if hour == 12 {
                                                hour = 0;
                                            }
                                        }
                                    }
                                    Rule::time_special => {
                                        if time_piece.as_str().eq_ignore_ascii_case("midnight") {
                                            hour = 0;
                                        } else if time_piece.as_str().eq_ignore_ascii_case("noon") {
                                            hour = 12;
                                        } else if time_piece.as_str().eq_ignore_ascii_case("now") {
                                            now = true;
                                        }
                                    }
                                    _ => unreachable!(),
                                }
                            }
                            if !now {
                                rv.time_spec = Some(TimeSpec::Abs {
                                    hour,
                                    minute,
                                    second,
                                });
                            }
                        }
                        Rule::date_absolute => {
                            let mut day = 0;
                            let mut month = None;
                            let mut year = None;
                            for date_piece in abs_time_piece.into_inner() {
                                match date_piece.as_rule() {
                                    Rule::english_date => {
                                        for english_piece in date_piece.into_inner() {
                                            match english_piece.as_rule() {
                                                Rule::english_month => {
                                                    month = Some(
                                                        match english_piece
                                                            .into_inner()
                                                            .next()
                                                            .unwrap()
                                                            .as_rule()
                                                        {
                                                            Rule::m01 => 1,
                                                            Rule::m02 => 2,
                                                            Rule::m03 => 3,
                                                            Rule::m04 => 4,
                                                            Rule::m05 => 5,
                                                            Rule::m06 => 6,
                                                            Rule::m07 => 7,
                                                            Rule::m08 => 8,
                                                            Rule::m09 => 9,
                                                            Rule::m10 => 10,
                                                            Rule::m11 => 11,
                                                            Rule::m12 => 12,
                                                            _ => unreachable!(),
                                                        },
                                                    );
                                                }
                                                Rule::english_day => {
                                                    day = english_piece.as_str()
                                                        [0..english_piece.as_str().len() - 2]
                                                        .parse()
                                                        .unwrap();
                                                }
                                                Rule::dd => {
                                                    day = english_piece
                                                        .as_str()
                                                        .parse::<i32>()
                                                        .unwrap();
                                                }
                                                Rule::yyyy => {
                                                    year = Some(
                                                        english_piece.as_str().parse().unwrap(),
                                                    );
                                                }
                                                _ => unreachable!(),
                                            }
                                        }
                                    }
                                    Rule::ddmmyyyy => {
                                        for date_piece in date_piece.into_inner() {
                                            match date_piece.as_rule() {
                                                Rule::dd => {
                                                    day =
                                                        date_piece.as_str().parse::<i32>().unwrap();
                                                }
                                                Rule::mm => {
                                                    month = Some(
                                                        date_piece.as_str().parse::<i32>().unwrap(),
                                                    );
                                                }
                                                Rule::yyyy => {
                                                    year =
                                                        Some(date_piece.as_str().parse().unwrap());
                                                }
                                                _ => unreachable!(),
                                            }
                                        }
                                    }
                                    _ => unreachable!(),
                                }
                            }
                            rv.date_spec = Some(DateSpec::Abs { day, month, year });
                        }
                        Rule::date_relative => {
                            let mut days = 0;
                            for days_piece in abs_time_piece.into_inner() {
                                match days_piece.as_rule() {
                                    Rule::tomorrow => {
                                        days = 1;
                                    }
                                    Rule::yesterday => {
                                        days = -1;
                                    }
                                    Rule::today => {
                                        days = 0;
                                    }
                                    Rule::in_days => {
                                        days = as_int(days_piece);
                                    }
                                    _ => unreachable!(),
                                }
                            }
                            rv.date_spec = Some(DateSpec::Rel { days });
                        }
                        _ => unreachable!(),
                    }
                }
            }
            Rule::rel_time | Rule::neg_rel_time => {
                let mut hours = 0;
                let mut minutes = 0;
                let mut seconds = 0;
                let is_negative = piece.as_rule() == Rule::neg_rel_time;
                for time_piece in piece.into_inner() {
                    match time_piece.as_rule() {
                        Rule::rel_hours => {
                            hours = as_int(time_piece);
                        }
                        Rule::rel_minutes => {
                            minutes = as_int(time_piece);
                        }
                        Rule::rel_seconds => {
                            seconds = as_int(time_piece);
                        }
                        _ => unreachable!(),
                    }
                }
                if is_negative {
                    hours *= -1;
                    minutes *= -1;
                    seconds *= -1;
                }
                rv.time_spec = Some(TimeSpec::Rel {
                    hours,
                    minutes,
                    seconds,
                });
            }
            _ => unreachable!(),
        }
    }

    // if unix time is used there is always an implied utc location
    // as this is the main thing that makes sense with unix timestamps
    if unix_time {
        if rv.locations.is_empty() || !find_zone(rv.locations[0]).map_or(false, |x| x.is_utc()) {
            rv.locations.insert(0, "utc");
        }
    }

    Ok(rv)
}
