//! 文本先绑定身份再生成统一 AST；限制解析资源，不绕过既有类型和流程编译。

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::domain::{Question, QuestionKind};
use crate::flow::{FlowBranch, FlowDefinition, FlowNode, FlowOutcome};
use crate::logic::{LogicError, LogicErrorCode, LogicValueType, check_raw_budget, error};
use crate::logic_document::{ComputedVariable, DocumentLogic, QuestionRule};

/// 文本错误携带源位置，语义错误同时保留原 AST 路径。
#[derive(Debug, thiserror::Error, Serialize)]
#[error("{error}（第 {line} 行，第 {column} 列）")]
pub struct LogicTextError {
    /// 稳定分类、题目身份和 AST 路径。
    #[serde(flatten)]
    #[source]
    pub error: LogicError,
    /// 从 1 开始的行号。
    pub line: usize,
    /// 从 1 开始的 Unicode 标量列号。
    pub column: usize,
}

fn located(source: &str, offset: usize, error: LogicError) -> LogicTextError {
    let prefix = &source[..offset];
    LogicTextError {
        line: prefix.bytes().filter(|byte| *byte == b'\n').count() + 1,
        column: prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1,
        error,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Word,
    Number,
    String,
    Symbol,
    End,
}

#[derive(Clone, Copy)]
struct Token<'a> {
    raw: &'a str,
    kind: Kind,
    offset: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Category {
    Question,
    Option,
    Variable,
    Node,
}

#[derive(Clone, Copy)]
struct Binding {
    category: Category,
    id: Uuid,
}

fn tokens(source: &str) -> Result<Vec<Token<'_>>, LogicTextError> {
    let fail = |offset, code| located(source, offset, error(code, "$", None));
    if source.len() > 1_048_576 {
        return Err(fail(0, LogicErrorCode::LimitExceeded));
    }
    let bytes = source.as_bytes();
    let mut result = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        if matches!(bytes[offset], b' ' | b'\t' | b'\r' | b'\n') {
            offset += 1;
            continue;
        }
        if source[offset..].starts_with("//") {
            offset += source[offset..].find('\n').unwrap_or(bytes.len() - offset);
            continue;
        }
        let start = offset;
        let kind = match bytes[offset] {
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                offset += 1;
                while bytes
                    .get(offset)
                    .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
                {
                    offset += 1;
                }
                if offset - start > 64 {
                    return Err(fail(start, LogicErrorCode::LimitExceeded));
                }
                Kind::Word
            }
            b'0'..=b'9' => {
                offset += 1;
                if bytes[start] != b'0' {
                    while bytes.get(offset).is_some_and(u8::is_ascii_digit) {
                        offset += 1;
                    }
                }
                if bytes.get(offset) == Some(&b'.')
                    && bytes.get(offset + 1).is_some_and(u8::is_ascii_digit)
                {
                    offset += 2;
                    while bytes.get(offset).is_some_and(u8::is_ascii_digit) {
                        offset += 1;
                    }
                }
                if matches!(bytes.get(offset), Some(b'e' | b'E')) {
                    let mut end = offset + 1;
                    if matches!(bytes.get(end), Some(b'+' | b'-')) {
                        end += 1;
                    }
                    if bytes.get(end).is_some_and(u8::is_ascii_digit) {
                        offset = end + 1;
                        while bytes.get(offset).is_some_and(u8::is_ascii_digit) {
                            offset += 1;
                        }
                    }
                }
                Kind::Number
            }
            b'"' => {
                offset += 1;
                while offset < bytes.len() && bytes[offset] != b'"' {
                    if bytes[offset] == b'\\' {
                        offset += 1;
                    }
                    offset += 1;
                }
                if offset >= bytes.len() {
                    return Err(fail(start, LogicErrorCode::InvalidSyntax));
                }
                offset += 1;
                if serde_json::from_str::<String>(&source[start..offset]).is_err() {
                    return Err(fail(start, LogicErrorCode::InvalidSyntax));
                }
                Kind::String
            }
            _ => {
                if ["->", "&&", "||", "==", "!=", "<=", ">="]
                    .iter()
                    .any(|op| source[offset..].starts_with(op))
                {
                    offset += 2;
                } else if b"{}()[],;:=!<>+*/-".contains(&bytes[offset]) {
                    offset += 1;
                } else {
                    return Err(fail(offset, LogicErrorCode::InvalidSyntax));
                }
                Kind::Symbol
            }
        };
        result.push(Token {
            raw: &source[start..offset],
            kind,
            offset: start,
        });
        if result.len() > 65_536 {
            return Err(fail(start, LogicErrorCode::LimitExceeded));
        }
    }
    result.push(Token {
        raw: "",
        kind: Kind::End,
        offset,
    });
    Ok(result)
}

struct Parser<'a> {
    source: &'a str,
    tokens: Vec<Token<'a>>,
    position: usize,
    bindings: BTreeMap<&'a str, Binding>,
    locations: Vec<(String, usize)>,
}

fn binary(raw: &str) -> Option<(u8, &'static str, &'static str)> {
    Some(match raw {
        "||" => (1, "any", ""),
        "&&" => (2, "all", ""),
        "==" => (3, "compare", "eq"),
        "!=" => (3, "compare", "ne"),
        "<" => (3, "compare", "lt"),
        "<=" => (3, "compare", "le"),
        ">" => (3, "compare", "gt"),
        ">=" => (3, "compare", "ge"),
        "+" => (4, "arithmetic", "add"),
        "-" => (4, "arithmetic", "subtract"),
        "*" => (5, "arithmetic", "multiply"),
        "/" => (5, "arithmetic", "divide"),
        _ => return None,
    })
}

impl<'a> Parser<'a> {
    fn token(&self) -> Token<'a> {
        self.tokens[self.position]
    }
    fn take(&mut self) -> Token<'a> {
        let token = self.token();
        if token.kind != Kind::End {
            self.position += 1;
        }
        token
    }
    fn failure(&self, code: LogicErrorCode, token: Token<'_>) -> LogicTextError {
        located(self.source, token.offset, error(code, "$", None))
    }
    fn accept(&mut self, raw: &str) -> bool {
        if self.token().raw != raw {
            return false;
        }
        self.take();
        true
    }
    fn expect(&mut self, raw: &str) -> Result<(), LogicTextError> {
        if self.accept(raw) {
            Ok(())
        } else {
            Err(self.failure(LogicErrorCode::InvalidSyntax, self.token()))
        }
    }
    fn word(&mut self) -> Result<&'a str, LogicTextError> {
        let token = self.take();
        if token.kind == Kind::Word {
            Ok(token.raw)
        } else {
            Err(self.failure(LogicErrorCode::InvalidSyntax, token))
        }
    }
    fn string(&mut self) -> Result<String, LogicTextError> {
        let token = self.take();
        if token.kind != Kind::String {
            return Err(self.failure(LogicErrorCode::InvalidSyntax, token));
        }
        let value: String = serde_json::from_str(token.raw)
            .map_err(|_| self.failure(LogicErrorCode::InvalidSyntax, token))?;
        if value.contains('\0') {
            return Err(self.failure(LogicErrorCode::InvalidSyntax, token));
        }
        Ok(value)
    }
    fn reference(&mut self, category: Category) -> Result<Uuid, LogicTextError> {
        let token = self.token();
        let alias = self.word()?;
        self.bindings
            .get(alias)
            .filter(|binding| binding.category == category)
            .map(|binding| binding.id)
            .ok_or_else(|| self.failure(LogicErrorCode::InvalidReference, token))
    }
    fn references(&mut self, category: Category) -> Result<Vec<Uuid>, LogicTextError> {
        self.expect("[")?;
        let mut ids = Vec::new();
        if !self.accept("]") {
            loop {
                ids.push(self.reference(category)?);
                if !self.accept(",") {
                    break;
                }
            }
            self.expect("]")?;
        }
        Ok(ids)
    }
    fn expression(&mut self, min: u8, depth: usize) -> Result<Value, LogicTextError> {
        if depth > 64 {
            return Err(self.failure(LogicErrorCode::LimitExceeded, self.token()));
        }
        let mut left = self.primary(depth)?;
        self.check_expression(&left)?;
        while let Some((precedence, op, kind)) = binary(self.token().raw) {
            if precedence < min {
                break;
            }
            self.take();
            let right = self.expression(precedence + 1, depth + 1)?;
            left = if matches!(op, "all" | "any") {
                json!({"op":op, "expressions":[left,right]})
            } else if op == "compare" {
                json!({"op":op, "comparison":kind, "left":left,"right":right})
            } else {
                json!({"op":op, "operator":kind, "left":left,"right":right})
            };
            self.check_expression(&left)?;
        }
        Ok(left)
    }
    fn check_expression(&self, value: &Value) -> Result<(), LogicTextError> {
        // 长中缀链也会加深 AST，必须在构造时拦截，避免超深值的析构耗尽栈。
        check_raw_budget(Some(value), 1, &mut 0)
            .map_err(|error| located(self.source, self.token().offset, error))
    }
    fn primary(&mut self, depth: usize) -> Result<Value, LogicTextError> {
        if self.accept("(") {
            let expression = self.expression(0, depth + 1)?;
            self.expect(")")?;
            return Ok(expression);
        }
        if self.accept("!") {
            return Ok(json!({"op":"not", "expression":self.expression(6, depth + 1)?}));
        }
        if self.accept("-") {
            let token = self.take();
            if token.kind != Kind::Number {
                return Err(self.failure(LogicErrorCode::InvalidSyntax, token));
            }
            return Ok(json!({"op":"number", "value":format!("-{}", token.raw)}));
        }
        let token = self.take();
        if token.kind == Kind::Number {
            return Ok(json!({"op":"number", "value":token.raw}));
        }
        if token.kind == Kind::String {
            self.position -= 1;
            return Ok(json!({"op":"text", "value":self.string()?}));
        }
        if matches!(token.raw, "true" | "false") {
            return Ok(json!({"op":"boolean", "value":token.raw == "true"}));
        }
        if token.kind != Kind::Word {
            return Err(self.failure(LogicErrorCode::InvalidSyntax, token));
        }
        if !self.accept("(") {
            let binding = self
                .bindings
                .get(token.raw)
                .filter(|b| b.category == Category::Variable)
                .ok_or_else(|| self.failure(LogicErrorCode::InvalidReference, token))?;
            return Ok(json!({"op":"variable", "variable_id":binding.id}));
        }
        let op = token.raw;
        let result = match op {
            "answer" | "answered" => {
                json!({"op":op,"question_id":self.reference(Category::Question)?})
            }
            "variable" => json!({"op":op,"variable_id":self.reference(Category::Variable)?}),
            "number" => json!({"op":op,"value":self.string()?}),
            "selected" | "contains_any" | "contains_all" => {
                let question_id = self.reference(Category::Question)?;
                self.expect(",")?;
                if op == "selected" {
                    json!({"op":op,"question_id":question_id,"option_id":self.reference(Category::Option)?})
                } else {
                    json!({"op":op,"question_id":question_id,"option_ids":self.references(Category::Option)?})
                }
            }
            "all" | "any" => {
                let mut expressions = Vec::new();
                if self.token().raw != ")" {
                    loop {
                        expressions.push(self.expression(0, depth + 1)?);
                        if !self.accept(",") {
                            break;
                        }
                    }
                }
                json!({"op":op,"expressions":expressions})
            }
            "coalesce" | "length" => {
                let value = self.expression(0, depth + 1)?;
                if op == "coalesce" {
                    self.expect(",")?;
                    json!({"op":op,"value":value,"fallback":self.expression(0, depth + 1)?})
                } else {
                    json!({"op":op,"value":value})
                }
            }
            _ => return Err(self.failure(LogicErrorCode::InvalidSyntax, token)),
        };
        self.expect(")")?;
        Ok(result)
    }
    fn parse(mut self, questions: &[Question]) -> Result<DocumentLogic, LogicTextError> {
        self.expect("dsl")?;
        let dsl = self.take();
        self.expect("semantics")?;
        let semantics = self.take();
        self.expect(";")?;
        if dsl.kind != Kind::Number || semantics.kind != Kind::Number {
            return Err(self.failure(LogicErrorCode::InvalidSyntax, dsl));
        }
        let mut logic = DocumentLogic {
            dsl_version: dsl
                .raw
                .parse()
                .map_err(|_| self.failure(LogicErrorCode::InvalidSyntax, dsl))?,
            semantics_version: semantics
                .raw
                .parse()
                .map_err(|_| self.failure(LogicErrorCode::InvalidSyntax, semantics))?,
            variables: Vec::new(),
            rules: Vec::new(),
            flow: None,
        };
        let mut bound_ids = BTreeSet::new();
        while self.accept("bind") {
            let token = self.token();
            let category = match self.word()? {
                "question" => Category::Question,
                "option" => Category::Option,
                "variable" => Category::Variable,
                "node" => Category::Node,
                _ => return Err(self.failure(LogicErrorCode::InvalidSyntax, token)),
            };
            let alias = self.word()?;
            self.expect("=")?;
            let raw = self.string()?;
            self.expect(";")?;
            let id = Uuid::parse_str(&raw)
                .map_err(|_| self.failure(LogicErrorCode::InvalidReference, token))?;
            if id.is_nil()
                || id.to_string() != raw
                || self.bindings.contains_key(alias)
                || !bound_ids.insert(id)
            {
                return Err(self.failure(LogicErrorCode::InvalidReference, token));
            }
            self.bindings.insert(alias, Binding { category, id });
        }
        let mut definitions = BTreeSet::new();
        let mut nodes = Vec::new();
        let mut entry = None;
        while self.token().kind != Kind::End {
            let token = self.take();
            match token.raw {
                "let" => {
                    let id = self.reference(Category::Variable)?;
                    if !definitions.insert(id) {
                        return Err(self.failure(LogicErrorCode::InvalidReference, token));
                    }
                    self.expect(":")?;
                    let value_type = match self.word()? {
                        "boolean" => LogicValueType::Boolean,
                        "number" => LogicValueType::Number,
                        "text" => LogicValueType::Text,
                        "choice" => LogicValueType::Choice,
                        "choices" => LogicValueType::Choices,
                        _ => return Err(self.failure(LogicErrorCode::InvalidSyntax, token)),
                    };
                    self.expect("=")?;
                    self.locations.push((
                        format!("$.logic.variables[{}]", logic.variables.len()),
                        token.offset,
                    ));
                    logic.variables.push(ComputedVariable {
                        id,
                        value_type,
                        expression: self.expression(0, 1)?,
                    });
                    self.expect(";")?;
                }
                "visible" | "required" | "validate" => {
                    let question_id = self.reference(Category::Question)?;
                    let index = if let Some(index) = logic
                        .rules
                        .iter()
                        .position(|rule| rule.question_id == question_id)
                    {
                        index
                    } else {
                        logic.rules.push(QuestionRule {
                            question_id,
                            visible_if: None,
                            required_if: None,
                            validate: None,
                        });
                        logic.rules.len() - 1
                    };
                    let (slot, key) = match token.raw {
                        "visible" => (&mut logic.rules[index].visible_if, "visible_if"),
                        "required" => (&mut logic.rules[index].required_if, "required_if"),
                        _ => (&mut logic.rules[index].validate, "validate"),
                    };
                    if slot.is_some() {
                        return Err(self.failure(LogicErrorCode::InvalidSyntax, token));
                    }
                    self.locations
                        .push((format!("$.logic.rules[{index}].{key}"), token.offset));
                    self.expect("=")?;
                    *slot = Some(self.expression(0, 1)?);
                    self.expect(";")?;
                }
                "entry" => {
                    if entry.is_some() {
                        return Err(self.failure(LogicErrorCode::InvalidSyntax, token));
                    }
                    entry = Some(self.reference(Category::Node)?);
                    self.expect(";")?;
                    self.locations.push(("$.logic.flow".into(), token.offset));
                }
                "page" | "branch" | "end" => {
                    let id = self.reference(Category::Node)?;
                    if !definitions.insert(id) {
                        return Err(self.failure(LogicErrorCode::InvalidReference, token));
                    }
                    self.locations
                        .push((format!("$.logic.flow.nodes[{}]", nodes.len()), token.offset));
                    nodes.push(match token.raw {
                        "page" => {
                            let question_ids = self.references(Category::Question)?;
                            self.expect("->")?;
                            let next = self.reference(Category::Node)?;
                            self.expect(";")?;
                            FlowNode::Page {
                                id,
                                question_ids,
                                next,
                            }
                        }
                        "end" => {
                            self.expect("=")?;
                            let outcome = match self.word()? {
                                "complete" => FlowOutcome::Complete,
                                "screen_out" => FlowOutcome::ScreenOut,
                                _ => return Err(self.failure(LogicErrorCode::InvalidSyntax, token)),
                            };
                            self.expect(";")?;
                            FlowNode::End { id, outcome }
                        }
                        _ => {
                            self.expect("{")?;
                            let mut branches = Vec::new();
                            while self.accept("when") {
                                let when = self.expression(0, 1)?;
                                self.expect("->")?;
                                let target = self.reference(Category::Node)?;
                                self.expect(";")?;
                                branches.push(FlowBranch { when, target });
                            }
                            self.expect("else")?;
                            self.expect("->")?;
                            let otherwise = self.reference(Category::Node)?;
                            self.expect(";")?;
                            self.expect("}")?;
                            FlowNode::Branch {
                                id,
                                branches,
                                otherwise,
                            }
                        }
                    });
                }
                _ => return Err(self.failure(LogicErrorCode::InvalidSyntax, token)),
            }
        }
        if !nodes.is_empty() || entry.is_some() {
            let entry =
                entry.ok_or_else(|| self.failure(LogicErrorCode::InvalidFlow, self.token()))?;
            logic.flow = Some(FlowDefinition { entry, nodes });
        }
        for binding in self.bindings.values() {
            let valid = match binding.category {
                Category::Question => questions.iter().any(|q| q.id == binding.id),
                Category::Option => questions.iter().any(|q| match &q.kind {
                    QuestionKind::SingleChoice { options }
                    | QuestionKind::MultipleChoice { options, .. } => {
                        options.iter().any(|o| o.id == binding.id)
                    }
                    _ => false,
                }),
                _ => definitions.contains(&binding.id),
            };
            if !valid {
                return Err(self.failure(LogicErrorCode::InvalidReference, self.tokens[0]));
            }
        }
        logic.compile(questions).map_err(|error| {
            let offset = self
                .locations
                .iter()
                .filter(|(path, _)| error.path.starts_with(path))
                .max_by_key(|(path, _)| path.len())
                .map_or(0, |(_, offset)| *offset);
            located(self.source, offset, error)
        })?;
        // 已通过支持版本检查；统一数值等价的 1、1.0、1e0 表示。
        logic.dsl_version = 1.into();
        logic.semantics_version = 1.into();
        Ok(logic)
    }
}

/// 解析绑定、表达式和流程，并通过原文档编译器检查后返回统一规则包。
///
/// # Errors
/// 语法、引用、资源或语义无效时返回带行列及 AST 路径的诊断。
pub fn parse_logic_text(
    source: &str,
    questions: &[Question],
) -> Result<DocumentLogic, LogicTextError> {
    Parser {
        source,
        tokens: tokens(source)?,
        position: 0,
        bindings: BTreeMap::new(),
        locations: Vec::new(),
    }
    .parse(questions)
}
