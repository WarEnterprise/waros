use std::collections::HashMap;

use crate::qasm::QasmError;
use crate::Circuit;

#[derive(Clone, Copy)]
struct Register {
    base: usize,
    size: usize,
}

pub(super) fn parse_qasm(source: &str) -> Result<Circuit, QasmError> {
    let statements = collect_statements(source)?;
    if statements.is_empty() {
        return Err(QasmError::ParseError {
            line: 1,
            message: "missing OPENQASM header".into(),
        });
    }
    if statements[0].1 != "OPENQASM 2.0" {
        return Err(QasmError::ParseError {
            line: statements[0].0,
            message: "expected 'OPENQASM 2.0;' header".into(),
        });
    }

    let mut qregs = HashMap::new();
    let mut cregs = HashMap::new();
    let mut total_qubits = 0usize;
    let mut total_classical_bits = 0usize;
    let mut operations = Vec::new();

    for (line, statement) in statements.into_iter().skip(1) {
        if statement.starts_with("include ") {
            if statement != "include \"qelib1.inc\"" {
                return Err(QasmError::ParseError {
                    line,
                    message: "only include \"qelib1.inc\" is supported".into(),
                });
            }
            continue;
        }

        if statement.starts_with("qreg ") {
            let (name, size) = parse_register_declaration(line, &statement, "qreg")?;
            qregs.insert(
                name,
                Register {
                    base: total_qubits,
                    size,
                },
            );
            total_qubits += size;
            continue;
        }

        if statement.starts_with("creg ") {
            let (name, size) = parse_register_declaration(line, &statement, "creg")?;
            cregs.insert(
                name,
                Register {
                    base: total_classical_bits,
                    size,
                },
            );
            total_classical_bits += size;
            continue;
        }

        operations.push((line, statement));
    }

    if total_qubits == 0 {
        return Err(QasmError::ParseError {
            line: 1,
            message: "at least one qreg declaration is required".into(),
        });
    }

    let mut circuit =
        Circuit::with_classical_bits(total_qubits, total_classical_bits).map_err(|error| {
            QasmError::ParseError {
                line: 1,
                message: error.to_string(),
            }
        })?;

    for (line, statement) in operations {
        if statement.starts_with("measure ") {
            parse_measurement(line, &statement, &qregs, &cregs, &mut circuit)?;
        } else if statement.starts_with("barrier ") {
            parse_barrier(line, &statement, &qregs, &mut circuit)?;
        } else {
            parse_gate(line, &statement, &qregs, &mut circuit)?;
        }
    }

    Ok(circuit)
}

fn collect_statements(source: &str) -> Result<Vec<(usize, String)>, QasmError> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut start_line = 1usize;

    for (index, raw_line) in source.lines().enumerate() {
        let line_number = index + 1;
        let line = raw_line.split("//").next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if current.is_empty() {
            start_line = line_number;
        } else {
            current.push(' ');
        }
        current.push_str(line);

        while let Some(semicolon) = current.find(';') {
            let statement = current[..semicolon].trim().to_string();
            if !statement.is_empty() {
                statements.push((start_line, statement));
            }
            current = current[semicolon + 1..].trim().to_string();
            start_line = line_number;
        }
    }

    if !current.trim().is_empty() {
        return Err(QasmError::ParseError {
            line: start_line,
            message: "missing semicolon".into(),
        });
    }

    Ok(statements)
}

fn parse_register_declaration(
    line: usize,
    statement: &str,
    keyword: &str,
) -> Result<(String, usize), QasmError> {
    let body = statement
        .strip_prefix(keyword)
        .ok_or_else(|| QasmError::ParseError {
            line,
            message: format!("expected '{keyword}' declaration"),
        })?
        .trim();
    let (name, index) = parse_indexed_identifier(line, body)?;
    Ok((name, index))
}

fn parse_measurement(
    line: usize,
    statement: &str,
    qregs: &HashMap<String, Register>,
    cregs: &HashMap<String, Register>,
    circuit: &mut Circuit,
) -> Result<(), QasmError> {
    let body = statement
        .strip_prefix("measure")
        .ok_or_else(|| QasmError::ParseError {
            line,
            message: "expected measurement".into(),
        })?
        .trim();
    let (qubit_ref, classical_ref) =
        body.split_once("->").ok_or_else(|| QasmError::ParseError {
            line,
            message: "measurement must use '->'".into(),
        })?;
    let qubit = resolve_register_reference(line, qubit_ref.trim(), qregs)?;
    let classical = resolve_register_reference(line, classical_ref.trim(), cregs)?;
    circuit
        .measure_into(qubit, classical)
        .map_err(|error| QasmError::ParseError {
            line,
            message: error.to_string(),
        })?;
    Ok(())
}

fn parse_barrier(
    line: usize,
    statement: &str,
    qregs: &HashMap<String, Register>,
    circuit: &mut Circuit,
) -> Result<(), QasmError> {
    let body = statement
        .strip_prefix("barrier")
        .ok_or_else(|| QasmError::ParseError {
            line,
            message: "expected barrier".into(),
        })?
        .trim();

    let mut qubits = Vec::new();
    for part in split_arguments(body) {
        let trimmed = part.trim();
        if trimmed.contains('[') {
            qubits.push(resolve_register_reference(line, trimmed, qregs)?);
        } else {
            let register = qregs
                .get(trimmed)
                .ok_or_else(|| QasmError::UndeclaredRegister(trimmed.to_string()))?;
            qubits.extend(register.base..(register.base + register.size));
        }
    }

    circuit
        .barrier(&qubits)
        .map_err(|error| QasmError::ParseError {
            line,
            message: error.to_string(),
        })?;
    Ok(())
}

fn parse_gate(
    line: usize,
    statement: &str,
    qregs: &HashMap<String, Register>,
    circuit: &mut Circuit,
) -> Result<(), QasmError> {
    let (head, operands) = split_gate_statement(line, statement)?;
    let (name, parameters) = parse_gate_head(line, head)?;
    let qubits = split_arguments(operands)
        .iter()
        .map(|operand| resolve_register_reference(line, operand.trim(), qregs))
        .collect::<Result<Vec<_>, _>>()?;

    match (name.as_str(), parameters.as_slice(), qubits.as_slice()) {
        ("h", [], [q0]) => circuit.h(*q0),
        ("x", [], [q0]) => circuit.x(*q0),
        ("y", [], [q0]) => circuit.y(*q0),
        ("z", [], [q0]) => circuit.z(*q0),
        ("s", [], [q0]) => circuit.s(*q0),
        ("sdg", [], [q0]) => circuit.sdg(*q0),
        ("t", [], [q0]) => circuit.t(*q0),
        ("tdg", [], [q0]) => circuit.tdg(*q0),
        ("rx", [theta], [q0]) => circuit.rx(*q0, *theta),
        ("ry", [theta], [q0]) => circuit.ry(*q0, *theta),
        ("rz", [theta], [q0]) => circuit.rz(*q0, *theta),
        ("u3", [theta, phi, lambda], [q0]) => circuit.u3(*q0, *theta, *phi, *lambda),
        ("cx", [], [q0, q1]) => circuit.cx(*q0, *q1),
        ("cz", [], [q0, q1]) => circuit.cz(*q0, *q1),
        ("swap", [], [q0, q1]) => circuit.swap(*q0, *q1),
        ("crk", [k], [q0, q1]) if is_integer(*k) => circuit.crk(*q0, *q1, *k as usize),
        ("ccx", [], [q0, q1, q2]) => circuit.toffoli(*q0, *q1, *q2),
        _ => return Err(QasmError::UnknownGate(name)),
    }
    .map_err(|error| QasmError::ParseError {
        line,
        message: error.to_string(),
    })?;

    Ok(())
}

fn is_integer(value: f64) -> bool {
    (value.round() - value).abs() < 1e-9 && value >= 0.0
}

fn split_gate_statement<'a>(
    line: usize,
    statement: &'a str,
) -> Result<(&'a str, &'a str), QasmError> {
    let mut depth = 0usize;
    for (index, character) in statement.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            character if character.is_whitespace() && depth == 0 => {
                let head = statement[..index].trim();
                let operands = statement[index..].trim();
                if operands.is_empty() {
                    break;
                }
                return Ok((head, operands));
            }
            _ => {}
        }
    }

    Err(QasmError::ParseError {
        line,
        message: "gate statement requires operands".into(),
    })
}

fn parse_gate_head(line: usize, head: &str) -> Result<(String, Vec<f64>), QasmError> {
    if let Some(open) = head.find('(') {
        let close = head.rfind(')').ok_or_else(|| QasmError::ParseError {
            line,
            message: "missing closing ')'".into(),
        })?;
        let name = head[..open].trim().to_ascii_lowercase();
        let parameters = split_arguments(&head[(open + 1)..close])
            .iter()
            .map(|argument| evaluate_expression(line, argument))
            .collect::<Result<Vec<_>, _>>()?;
        Ok((name, parameters))
    } else {
        Ok((head.trim().to_ascii_lowercase(), Vec::new()))
    }
}

fn resolve_register_reference(
    line: usize,
    reference: &str,
    registers: &HashMap<String, Register>,
) -> Result<usize, QasmError> {
    let (name, index) = parse_indexed_identifier(line, reference)?;
    let register = registers
        .get(&name)
        .ok_or_else(|| QasmError::UndeclaredRegister(name.clone()))?;
    if index >= register.size {
        return Err(QasmError::QubitOutOfRange {
            register: name,
            index,
            size: register.size,
        });
    }
    Ok(register.base + index)
}

fn parse_indexed_identifier(line: usize, source: &str) -> Result<(String, usize), QasmError> {
    let open = source.find('[').ok_or_else(|| QasmError::ParseError {
        line,
        message: format!("expected indexed identifier, got '{source}'"),
    })?;
    let close = source.rfind(']').ok_or_else(|| QasmError::ParseError {
        line,
        message: format!("expected closing ']' in '{source}'"),
    })?;
    let name = source[..open].trim().to_string();
    let index = source[(open + 1)..close]
        .trim()
        .parse::<usize>()
        .map_err(|_| QasmError::ParseError {
            line,
            message: format!("invalid register index in '{source}'"),
        })?;
    Ok((name, index))
}

fn split_arguments(source: &str) -> Vec<String> {
    let mut arguments = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for character in source.chars() {
        match character {
            ',' if depth == 0 => {
                if !current.trim().is_empty() {
                    arguments.push(current.trim().to_string());
                }
                current.clear();
            }
            '(' => {
                depth += 1;
                current.push(character);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(character);
            }
            _ => current.push(character),
        }
    }
    if !current.trim().is_empty() {
        arguments.push(current.trim().to_string());
    }
    arguments
}

fn evaluate_expression(line: usize, source: &str) -> Result<f64, QasmError> {
    let mut parser = ExpressionParser::new(source, line);
    let value = parser.parse_expression()?;
    parser.skip_whitespace();
    if !parser.is_eof() {
        return Err(QasmError::ParseError {
            line,
            message: format!("unexpected token in expression '{}'", source.trim()),
        });
    }
    Ok(value)
}

struct ExpressionParser<'a> {
    input: &'a str,
    line: usize,
    position: usize,
}

impl<'a> ExpressionParser<'a> {
    fn new(input: &'a str, line: usize) -> Self {
        Self {
            input,
            line,
            position: 0,
        }
    }

    fn parse_expression(&mut self) -> Result<f64, QasmError> {
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

    fn parse_term(&mut self) -> Result<f64, QasmError> {
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

    fn parse_factor(&mut self) -> Result<f64, QasmError> {
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
                return Err(QasmError::ParseError {
                    line: self.line,
                    message: "missing closing ')' in expression".into(),
                });
            }
            return Ok(value);
        }
        if self.starts_with("pi") {
            self.position += 2;
            return Ok(std::f64::consts::PI);
        }
        self.parse_number()
    }

    fn parse_number(&mut self) -> Result<f64, QasmError> {
        let start = self.position;
        while let Some(character) = self.peek() {
            if character.is_ascii_digit() || matches!(character, '.' | 'e' | 'E' | '+') {
                self.position += 1;
            } else if character == '-' && self.position > start {
                self.position += 1;
            } else {
                break;
            }
        }
        self.input[start..self.position]
            .trim()
            .parse::<f64>()
            .map_err(|_| QasmError::ParseError {
                line: self.line,
                message: format!(
                    "invalid numeric literal '{}'",
                    &self.input[start..self.position]
                ),
            })
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

    fn starts_with(&self, prefix: &str) -> bool {
        self.input[self.position..].starts_with(prefix)
    }

    fn peek(&self) -> Option<char> {
        self.input[self.position..].chars().next()
    }

    fn is_eof(&self) -> bool {
        self.position >= self.input.len()
    }
}
