#![allow(clippy::cast_sign_loss)]

use std::collections::HashMap;

use crate::gate;
use crate::gate::Gate;
use crate::qasm::QasmError;
use crate::Circuit;

#[derive(Clone, Copy)]
struct Register {
    base: usize,
    size: usize,
}

#[derive(Clone)]
struct GateDefinition {
    parameters: Vec<String>,
    qubits: Vec<String>,
    body: Vec<String>,
}

struct ParseContext<'a> {
    qregs: &'a HashMap<String, Register>,
    custom_gates: &'a HashMap<String, GateDefinition>,
    qubit_scope: &'a HashMap<String, usize>,
    parameter_scope: &'a HashMap<String, f64>,
}

struct GateApplication {
    gate: Gate,
    targets: Vec<usize>,
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
    let mut custom_gates = HashMap::new();
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

        if statement.starts_with("gate ") {
            let definition = parse_gate_definition(line, &statement)?;
            custom_gates.insert(definition.0, definition.1);
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

    let empty_qubit_scope = HashMap::new();
    let empty_parameter_scope = HashMap::new();
    let context = ParseContext {
        qregs: &qregs,
        custom_gates: &custom_gates,
        qubit_scope: &empty_qubit_scope,
        parameter_scope: &empty_parameter_scope,
    };

    for (line, statement) in operations {
        parse_statement(line, &statement, &cregs, &context, &mut circuit)?;
    }

    Ok(circuit)
}

fn parse_statement(
    line: usize,
    statement: &str,
    cregs: &HashMap<String, Register>,
    context: &ParseContext<'_>,
    circuit: &mut Circuit,
) -> Result<(), QasmError> {
    if statement.starts_with("measure ") {
        parse_measurement(line, statement, context.qregs, cregs, circuit)?;
    } else if statement.starts_with("barrier ") {
        parse_barrier(line, statement, context.qregs, context.qubit_scope, circuit)?;
    } else if statement.starts_with("if(") || statement.starts_with("if (") {
        parse_conditional(line, statement, cregs, context, circuit)?;
    } else {
        for application in expand_gate(line, statement, context)? {
            circuit
                .custom_gate(application.gate, &application.targets)
                .map_err(|error| QasmError::ParseError {
                    line,
                    message: error.to_string(),
                })?;
        }
    }

    Ok(())
}

fn collect_statements(source: &str) -> Result<Vec<(usize, String)>, QasmError> {
    let sanitized = source
        .lines()
        .map(|line| line.split("//").next().unwrap_or("").trim_end())
        .collect::<Vec<_>>()
        .join("\n");

    let mut statements = Vec::new();
    let mut current = String::new();
    let mut start_line = 1usize;
    let mut line = 1usize;
    let mut brace_depth = 0usize;

    for character in sanitized.chars() {
        if current.is_empty() && !character.is_whitespace() {
            start_line = line;
        }

        match character {
            '\n' => {
                if !current.ends_with(' ') && !current.is_empty() {
                    current.push(' ');
                }
                line += 1;
            }
            '{' => {
                brace_depth += 1;
                current.push(character);
            }
            '}' => {
                brace_depth = brace_depth.saturating_sub(1);
                current.push(character);
                if brace_depth == 0 {
                    push_statement(&mut statements, start_line, &mut current);
                }
            }
            ';' if brace_depth == 0 => push_statement(&mut statements, start_line, &mut current),
            _ => current.push(character),
        }
    }

    if !current.trim().is_empty() {
        return Err(QasmError::ParseError {
            line: start_line,
            message: "missing semicolon or closing '}'".into(),
        });
    }

    Ok(statements)
}

fn push_statement(statements: &mut Vec<(usize, String)>, line: usize, current: &mut String) {
    let statement = current.trim().to_string();
    if !statement.is_empty() {
        statements.push((line, statement));
    }
    current.clear();
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
    parse_indexed_identifier(line, body)
}

fn parse_gate_definition(
    line: usize,
    statement: &str,
) -> Result<(String, GateDefinition), QasmError> {
    let body = statement
        .strip_prefix("gate")
        .ok_or_else(|| QasmError::ParseError {
            line,
            message: "expected gate definition".into(),
        })?
        .trim();
    let (header, body) = body.split_once('{').ok_or_else(|| QasmError::ParseError {
        line,
        message: "gate definition must contain '{'".into(),
    })?;
    let gate_body = body.trim().trim_end_matches('}').trim();
    let (head, operands) = split_gate_statement(line, header.trim())?;
    let (name, parameters) = parse_identifier_list(line, head)?;
    let qubits = split_arguments(operands);
    let body = gate_body
        .split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    Ok((
        name,
        GateDefinition {
            parameters,
            qubits,
            body,
        },
    ))
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
    let qubit = resolve_quantum_reference(line, qubit_ref.trim(), qregs, &HashMap::new())?;
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
    qubit_scope: &HashMap<String, usize>,
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
        if let Some(qubit) = qubit_scope.get(part.trim()) {
            qubits.push(*qubit);
        } else if part.contains('[') {
            qubits.push(resolve_quantum_reference(
                line,
                part.trim(),
                qregs,
                qubit_scope,
            )?);
        } else {
            let register = qregs
                .get(part.trim())
                .ok_or_else(|| QasmError::UndeclaredRegister(part.trim().to_string()))?;
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

fn parse_conditional(
    line: usize,
    statement: &str,
    cregs: &HashMap<String, Register>,
    context: &ParseContext<'_>,
    circuit: &mut Circuit,
) -> Result<(), QasmError> {
    let open = statement.find('(').ok_or_else(|| QasmError::ParseError {
        line,
        message: "conditional statement missing '('".into(),
    })?;
    let close = statement.find(')').ok_or_else(|| QasmError::ParseError {
        line,
        message: "conditional statement missing ')'".into(),
    })?;
    let condition = statement[(open + 1)..close].trim();
    let operation = statement[(close + 1)..].trim();
    let (register_name, expected_value) =
        condition
            .split_once("==")
            .ok_or_else(|| QasmError::ParseError {
                line,
                message: "conditional must use '=='".into(),
            })?;
    let register = cregs
        .get(register_name.trim())
        .ok_or_else(|| QasmError::UndeclaredRegister(register_name.trim().to_string()))?;
    let expected_value =
        expected_value
            .trim()
            .parse::<usize>()
            .map_err(|_| QasmError::ParseError {
                line,
                message: "conditional value must be an integer".into(),
            })?;

    for application in expand_gate(line, operation, context)? {
        circuit
            .conditional_gate(
                &(register.base..(register.base + register.size)).collect::<Vec<_>>(),
                expected_value,
                application.gate,
                &application.targets,
            )
            .map_err(|error| QasmError::ParseError {
                line,
                message: error.to_string(),
            })?;
    }

    Ok(())
}

fn expand_gate(
    line: usize,
    statement: &str,
    context: &ParseContext<'_>,
) -> Result<Vec<GateApplication>, QasmError> {
    let (head, operands) = split_gate_statement(line, statement)?;
    let (name, parameters) = parse_gate_head(line, head, context.parameter_scope)?;
    let qubits = split_arguments(operands)
        .iter()
        .map(|operand| {
            resolve_quantum_reference(line, operand.trim(), context.qregs, context.qubit_scope)
        })
        .collect::<Result<Vec<_>, _>>()?;

    if let Some(application) = builtin_gate(name.as_str(), &parameters, &qubits) {
        return application.map_or_else(
            |error| {
                Err(QasmError::ParseError {
                    line,
                    message: error,
                })
            },
            Ok,
        );
    }

    let definition = context
        .custom_gates
        .get(name.as_str())
        .ok_or_else(|| QasmError::UnknownGate(name.clone()))?;
    if definition.parameters.len() != parameters.len() {
        return Err(QasmError::ParseError {
            line,
            message: format!(
                "gate '{}' expects {} parameters, got {}",
                name,
                definition.parameters.len(),
                parameters.len()
            ),
        });
    }
    if definition.qubits.len() != qubits.len() {
        return Err(QasmError::ParseError {
            line,
            message: format!(
                "gate '{}' expects {} operands, got {}",
                name,
                definition.qubits.len(),
                qubits.len()
            ),
        });
    }

    let mut parameter_scope = context.parameter_scope.clone();
    for (parameter_name, value) in definition.parameters.iter().zip(parameters.iter().copied()) {
        parameter_scope.insert(parameter_name.clone(), value);
    }

    let mut qubit_scope = context.qubit_scope.clone();
    for (qubit_name, qubit) in definition.qubits.iter().zip(qubits.iter().copied()) {
        qubit_scope.insert(qubit_name.clone(), qubit);
    }

    let nested_context = ParseContext {
        qregs: context.qregs,
        custom_gates: context.custom_gates,
        qubit_scope: &qubit_scope,
        parameter_scope: &parameter_scope,
    };

    let mut expanded = Vec::new();
    for nested_statement in &definition.body {
        expanded.extend(expand_gate(line, nested_statement, &nested_context)?);
    }
    Ok(expanded)
}

fn builtin_gate(
    name: &str,
    parameters: &[f64],
    qubits: &[usize],
) -> Option<Result<Vec<GateApplication>, String>> {
    let single = |gate: Gate, qubit: usize| {
        Ok(vec![GateApplication {
            gate,
            targets: vec![qubit],
        }])
    };
    let double = |gate: Gate, q0: usize, q1: usize| {
        Ok(vec![GateApplication {
            gate,
            targets: vec![q0, q1],
        }])
    };

    Some(match (name, parameters, qubits) {
        ("id", [], [_q0]) => Ok(Vec::new()),
        ("h", [], [q0]) => single(gate::h(), *q0),
        ("x", [], [q0]) => single(gate::x(), *q0),
        ("y", [], [q0]) => single(gate::y(), *q0),
        ("z", [], [q0]) => single(gate::z(), *q0),
        ("s", [], [q0]) => single(gate::s(), *q0),
        ("sdg", [], [q0]) => single(gate::sdg(), *q0),
        ("t", [], [q0]) => single(gate::t(), *q0),
        ("tdg", [], [q0]) => single(gate::tdg(), *q0),
        ("rx", [theta], [q0]) => single(gate::rx(*theta), *q0),
        ("ry", [theta], [q0]) => single(gate::ry(*theta), *q0),
        ("rz", [theta], [q0]) => single(gate::rz(*theta), *q0),
        ("u1", [lambda], [q0]) => single(gate::rz(*lambda), *q0),
        ("u2", [phi, lambda], [q0]) => {
            single(gate::u3(std::f64::consts::FRAC_PI_2, *phi, *lambda), *q0)
        }
        ("u" | "u3", [theta, phi, lambda], [q0]) => single(gate::u3(*theta, *phi, *lambda), *q0),
        ("cx", [], [q0, q1]) => double(gate::cnot(), *q0, *q1),
        ("cz", [], [q0, q1]) => double(gate::cz(), *q0, *q1),
        ("swap", [], [q0, q1]) => double(gate::swap(), *q0, *q1),
        ("crk", [k], [q0, q1]) if is_integer(*k) => double(gate::crk(*k as usize), *q0, *q1),
        ("ccx", [], [q0, q1, q2]) => Ok(toffoli_decomposition(*q0, *q1, *q2)),
        _ => return None,
    })
}

fn toffoli_decomposition(
    control_0: usize,
    control_1: usize,
    target: usize,
) -> Vec<GateApplication> {
    let mut applications = Vec::new();
    let push_1q = |gate: Gate, qubit: usize, operations: &mut Vec<GateApplication>| {
        operations.push(GateApplication {
            gate,
            targets: vec![qubit],
        });
    };
    let push_2q = |gate: Gate, q0: usize, q1: usize, operations: &mut Vec<GateApplication>| {
        operations.push(GateApplication {
            gate,
            targets: vec![q0, q1],
        });
    };

    push_1q(gate::h(), target, &mut applications);
    push_2q(gate::cnot(), control_1, target, &mut applications);
    push_1q(gate::tdg(), target, &mut applications);
    push_2q(gate::cnot(), control_0, target, &mut applications);
    push_1q(gate::t(), target, &mut applications);
    push_2q(gate::cnot(), control_1, target, &mut applications);
    push_1q(gate::tdg(), target, &mut applications);
    push_2q(gate::cnot(), control_0, target, &mut applications);
    push_1q(gate::t(), control_1, &mut applications);
    push_1q(gate::t(), target, &mut applications);
    push_1q(gate::h(), target, &mut applications);
    push_2q(gate::cnot(), control_0, control_1, &mut applications);
    push_1q(gate::t(), control_0, &mut applications);
    push_1q(gate::tdg(), control_1, &mut applications);
    push_2q(gate::cnot(), control_0, control_1, &mut applications);
    applications
}

fn is_integer(value: f64) -> bool {
    (value.round() - value).abs() < 1e-9 && value >= 0.0
}

fn split_gate_statement(line: usize, statement: &str) -> Result<(&str, &str), QasmError> {
    let mut depth = 0usize;
    for (index, character) in statement.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            character if character.is_whitespace() && depth == 0 => {
                let head = statement[..index].trim();
                let operands = statement[index..].trim();
                if !operands.is_empty() {
                    return Ok((head, operands));
                }
            }
            _ => {}
        }
    }

    Err(QasmError::ParseError {
        line,
        message: "gate statement requires operands".into(),
    })
}

fn parse_gate_head(
    line: usize,
    head: &str,
    parameter_scope: &HashMap<String, f64>,
) -> Result<(String, Vec<f64>), QasmError> {
    if let Some(open) = head.find('(') {
        let close = head.rfind(')').ok_or_else(|| QasmError::ParseError {
            line,
            message: "missing closing ')'".into(),
        })?;
        let name = head[..open].trim().to_ascii_lowercase();
        let parameters = split_arguments(&head[(open + 1)..close])
            .iter()
            .map(|argument| evaluate_expression(line, argument, parameter_scope))
            .collect::<Result<Vec<_>, _>>()?;
        Ok((name, parameters))
    } else {
        Ok((head.trim().to_ascii_lowercase(), Vec::new()))
    }
}

fn parse_identifier_list(line: usize, source: &str) -> Result<(String, Vec<String>), QasmError> {
    if let Some(open) = source.find('(') {
        let close = source.rfind(')').ok_or_else(|| QasmError::ParseError {
            line,
            message: "missing closing ')'".into(),
        })?;
        Ok((
            source[..open].trim().to_ascii_lowercase(),
            split_arguments(&source[(open + 1)..close]),
        ))
    } else {
        Ok((source.trim().to_ascii_lowercase(), Vec::new()))
    }
}

fn resolve_quantum_reference(
    line: usize,
    reference: &str,
    registers: &HashMap<String, Register>,
    qubit_scope: &HashMap<String, usize>,
) -> Result<usize, QasmError> {
    if let Some(qubit) = qubit_scope.get(reference) {
        return Ok(*qubit);
    }
    resolve_register_reference(line, reference, registers)
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

fn evaluate_expression(
    line: usize,
    source: &str,
    scope: &HashMap<String, f64>,
) -> Result<f64, QasmError> {
    let mut parser = ExpressionParser::new(source, line, scope);
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
    scope: &'a HashMap<String, f64>,
}

impl<'a> ExpressionParser<'a> {
    fn new(input: &'a str, line: usize, scope: &'a HashMap<String, f64>) -> Self {
        Self {
            input,
            line,
            position: 0,
            scope,
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
        if let Some(identifier) = self.parse_identifier() {
            if let Some(value) = self.scope.get(identifier.as_str()) {
                return Ok(*value);
            }
            return Err(QasmError::ParseError {
                line: self.line,
                message: format!("unknown identifier '{identifier}'"),
            });
        }
        self.parse_number()
    }

    fn parse_identifier(&mut self) -> Option<String> {
        self.skip_whitespace();
        let start = self.position;
        while let Some(character) = self.peek() {
            if character.is_ascii_alphabetic() || character == '_' {
                self.position += 1;
            } else {
                break;
            }
        }
        (self.position > start).then(|| self.input[start..self.position].to_string())
    }

    fn parse_number(&mut self) -> Result<f64, QasmError> {
        let start = self.position;
        while let Some(character) = self.peek() {
            if character.is_ascii_digit()
                || character == '-' && self.position > start
                || matches!(character, '.' | 'e' | 'E' | '+')
            {
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
