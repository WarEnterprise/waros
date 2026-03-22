use std::fs;
use std::path::Path;

use waros_quantum::{NoiseModel, Simulator};

pub type CliResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

pub fn read_utf8(path: &Path) -> CliResult<String> {
    Ok(fs::read_to_string(path)?)
}

pub fn build_simulator(noise: &str, seed: Option<u64>) -> CliResult<Simulator> {
    let builder = match seed {
        Some(seed) => Simulator::builder().seed(seed),
        None => Simulator::builder(),
    };
    let noise_model = parse_noise_model(noise)?;
    Ok(builder.noise(noise_model).build())
}

pub fn parse_noise_model(raw: &str) -> CliResult<NoiseModel> {
    match raw.to_ascii_lowercase().as_str() {
        "ideal" => Ok(NoiseModel::ideal()),
        "ibm" => Ok(NoiseModel::ibm_like()),
        "ionq" => Ok(NoiseModel::ionq_like()),
        _ => parse_custom_noise(raw),
    }
}

fn parse_custom_noise(raw: &str) -> CliResult<NoiseModel> {
    let values = raw
        .strip_prefix("custom:")
        .ok_or_else(|| format!("unsupported noise profile '{raw}'"))?;
    let parts = values
        .split(',')
        .map(str::trim)
        .map(str::parse::<f64>)
        .collect::<Result<Vec<_>, _>>()?;
    if parts.len() != 3 {
        return Err("custom noise must be 'custom:single,two,readout'".into());
    }
    Ok(NoiseModel::uniform(parts[0], parts[1], parts[2]))
}

pub fn parse_angle(raw: &str) -> CliResult<f64> {
    let mut parser = AngleParser::new(raw);
    let value = parser.parse_expression()?;
    parser.skip_whitespace();
    if !parser.is_eof() {
        return Err(format!("invalid angle expression '{raw}'").into());
    }
    Ok(value)
}

struct AngleParser<'a> {
    input: &'a str,
    position: usize,
}

impl<'a> AngleParser<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, position: 0 }
    }

    fn parse_expression(&mut self) -> CliResult<f64> {
        let mut value = self.parse_term()?;
        loop {
            self.skip_whitespace();
            if self.consume('+') {
                value += self.parse_term()?;
            } else if self.consume('-') {
                value -= self.parse_term()?;
            } else {
                break;
            }
        }
        Ok(value)
    }

    fn parse_term(&mut self) -> CliResult<f64> {
        let mut value = self.parse_factor()?;
        loop {
            self.skip_whitespace();
            if self.consume('*') {
                value *= self.parse_factor()?;
            } else if self.consume('/') {
                value /= self.parse_factor()?;
            } else {
                break;
            }
        }
        Ok(value)
    }

    fn parse_factor(&mut self) -> CliResult<f64> {
        self.skip_whitespace();
        if self.consume('+') {
            return self.parse_factor();
        }
        if self.consume('-') {
            return Ok(-self.parse_factor()?);
        }
        if self.consume('(') {
            let value = self.parse_expression()?;
            if !self.consume(')') {
                return Err("missing ')' in angle expression".into());
            }
            return Ok(value);
        }
        if self.input[self.position..].starts_with("pi") {
            self.position += 2;
            return Ok(std::f64::consts::PI);
        }
        self.parse_number()
    }

    fn parse_number(&mut self) -> CliResult<f64> {
        let start = self.position;
        while let Some(character) = self.peek() {
            if character.is_ascii_digit() || matches!(character, '.' | 'e' | 'E' | '+' | '-') {
                self.position += 1;
            } else {
                break;
            }
        }
        Ok(self.input[start..self.position].parse::<f64>()?)
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(character) if character.is_whitespace()) {
            self.position += 1;
        }
    }

    fn consume(&mut self, expected: char) -> bool {
        self.skip_whitespace();
        if self.peek() == Some(expected) {
            self.position += expected.len_utf8();
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<char> {
        self.input[self.position..].chars().next()
    }

    fn is_eof(&self) -> bool {
        self.position >= self.input.len()
    }
}
