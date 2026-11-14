// SPDX-License-Identifier: AGPL-3.0-or-later
use super::{Error, Program, UsedVars, Variable};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Op {
    Add,
    Mul,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ClockPart {
    Hour,
    Weekday,
    ShanghaiDate,
}
#[derive(Clone, Debug)]
pub(super) enum Node {
    Number(f64),
    Var(Variable),
    Binary(Op, Box<Node>, Box<Node>),
    Not(Box<Node>),
    Conditional(Box<Node>, Box<Node>, Box<Node>),
    Tier {
        #[cfg(test)]
        label: String,
        body: Box<Node>,
    },
    Clock(ClockPart),
}
#[derive(Clone, Debug, PartialEq)]
enum Token {
    Number(f64),
    Id(String),
    String(String),
    Op(Op),
    Not,
    Lp,
    Rp,
    Comma,
    Question,
    Colon,
    End,
}

fn lex(source: &str) -> Result<Vec<Token>, Error> {
    if source.is_empty() || source.len() > 16_384 {
        return Err(Error::Limit);
    }
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        let token = match bytes[i] {
            b'0'..=b'9' | b'.' => {
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                if i < bytes.len() && bytes[i] == b'.' {
                    i += 1;
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                let n = source[start..i].parse::<f64>().map_err(|_| Error::Syntax)?;
                if !n.is_finite() {
                    return Err(Error::Unsupported);
                }
                Token::Number(n)
            }
            b'a'..=b'z' | b'_' => {
                i += 1;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                Token::Id(source[start..i].to_owned())
            }
            b'"' => {
                i += 1;
                let left = i;
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] < 32 || bytes[i] == b'\\' {
                        return Err(Error::Unsupported);
                    }
                    i += 1;
                }
                if i == bytes.len() || i - left > 128 || i == left {
                    return Err(Error::Syntax);
                }
                let text = source[left..i].to_owned();
                i += 1;
                Token::String(text)
            }
            _ => {
                i += 1;
                match bytes[start] {
                    b'+' => Token::Op(Op::Add),
                    b'*' => Token::Op(Op::Mul),
                    b'<' | b'>' => {
                        let equal = i < bytes.len() && bytes[i] == b'=';
                        if equal {
                            i += 1;
                        }
                        Token::Op(match (bytes[start], equal) {
                            (b'<', false) => Op::Lt,
                            (b'<', true) => Op::Le,
                            (b'>', false) => Op::Gt,
                            _ => Op::Ge,
                        })
                    }
                    b'&' | b'|' if i < bytes.len() && bytes[i] == bytes[start] => {
                        i += 1;
                        Token::Op(if bytes[start] == b'&' {
                            Op::And
                        } else {
                            Op::Or
                        })
                    }
                    b'!' => Token::Not,
                    b'(' => Token::Lp,
                    b')' => Token::Rp,
                    b',' => Token::Comma,
                    b'?' => Token::Question,
                    b':' => Token::Colon,
                    _ => return Err(Error::Unsupported),
                }
            }
        };
        out.push(token);
        if out.len() > 2048 {
            return Err(Error::Limit);
        }
    }
    out.push(Token::End);
    Ok(out)
}
struct Parser {
    tokens: Vec<Token>,
    at: usize,
    nodes: usize,
    used: UsedVars,
    calendar: bool,
    tiers: usize,
}
impl Parser {
    fn take(&mut self) -> Token {
        let v = self.tokens[self.at].clone();
        self.at += 1;
        v
    }
    fn want(&mut self, token: Token) -> Result<(), Error> {
        if self.tokens.get(self.at) != Some(&token) {
            return Err(Error::Syntax);
        }
        self.at += 1;
        Ok(())
    }
    fn expr(&mut self, min: u8, depth: usize) -> Result<Node, Error> {
        if depth > 64 {
            return Err(Error::Limit);
        }
        self.nodes += 1;
        if self.nodes > 512 {
            return Err(Error::Limit);
        }
        let mut lhs = match self.take() {
            Token::Number(v) => Node::Number(v),
            Token::Lp => {
                let n = self.expr(0, depth + 1)?;
                self.want(Token::Rp)?;
                n
            }
            Token::Not => Node::Not(Box::new(self.expr(7, depth + 1)?)),
            Token::Id(name) => {
                if let Some(var) = Variable::parse(&name) {
                    self.used.insert(var);
                    Node::Var(var)
                } else {
                    self.want(Token::Lp)?;
                    let Token::String(text) = self.take() else {
                        return Err(Error::Syntax);
                    };
                    if name == "tier" {
                        self.tiers += 1;
                        if self.tiers > 4 {
                            return Err(Error::Limit);
                        }
                        self.want(Token::Comma)?;
                        let value = self.expr(0, depth + 1)?;
                        self.want(Token::Rp)?;
                        Node::Tier {
                            #[cfg(test)]
                            label: text,
                            body: Box::new(value),
                        }
                    } else {
                        self.want(Token::Rp)?;
                        self.calendar = true;
                        Node::Clock(match (name.as_str(), text.as_str()) {
                            ("hour", "UTC") => ClockPart::Hour,
                            ("weekday", "UTC") => ClockPart::Weekday,
                            ("date", "Asia/Shanghai") => ClockPart::ShanghaiDate,
                            _ => return Err(Error::Unsupported),
                        })
                    }
                }
            }
            _ => return Err(Error::Syntax),
        };
        loop {
            if self.tokens.get(self.at) == Some(&Token::Question) && min == 0 {
                self.at += 1;
                let yes = self.expr(0, depth + 1)?;
                self.want(Token::Colon)?;
                let no = self.expr(0, depth + 1)?;
                lhs = Node::Conditional(Box::new(lhs), Box::new(yes), Box::new(no));
                continue;
            }
            let Some(Token::Op(op)) = self.tokens.get(self.at) else {
                break;
            };
            let op = *op;
            let power = match op {
                Op::Or => 1,
                Op::And => 2,
                Op::Lt | Op::Le | Op::Gt | Op::Ge => 3,
                Op::Add => 4,
                Op::Mul => 5,
            };
            if power < min {
                break;
            }
            self.at += 1;
            let rhs = self.expr(power + 1, depth + 1)?;
            lhs = Node::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }
}
fn scalar(node: &Node) -> bool {
    matches!(node, Node::Number(_))
        || matches!(node,Node::Binary(Op::Mul,a,b) if scalar(a)&&scalar(b))
}
fn linear(node: &Node) -> bool {
    match node {
        Node::Number(n) => *n == 0.0,
        Node::Var(v) => *v != Variable::Len,
        Node::Binary(Op::Add, a, b) => linear(a) && linear(b),
        Node::Binary(Op::Mul, a, b) => (scalar(a) && linear(b)) || (linear(a) && scalar(b)),
        _ => false,
    }
}
fn predicate(node: &Node) -> bool {
    match node {
        Node::Binary(Op::And | Op::Or, a, b) => predicate(a) && predicate(b),
        Node::Not(n) => predicate(n),
        Node::Binary(Op::Lt | Op::Le | Op::Gt | Op::Ge, a, b) => {
            matches!(&**a, Node::Var(Variable::Len) | Node::Clock(_))
                && matches!(&**b, Node::Number(_))
        }
        _ => false,
    }
}
fn root_shape(node: &Node) -> bool {
    match node {
        Node::Tier { body, .. } => linear(body),
        Node::Conditional(p, a, b) => predicate(p) && root_shape(a) && root_shape(b),
        Node::Binary(Op::Mul, a, b) => (scalar(a) && root_shape(b)) || (root_shape(a) && scalar(b)),
        _ => false,
    }
}
pub(super) fn compile(source: &str) -> Result<Program, Error> {
    let mut p = Parser {
        tokens: lex(source)?,
        at: 0,
        nodes: 0,
        used: UsedVars::default(),
        calendar: false,
        tiers: 0,
    };
    let root = p.expr(0, 0)?;
    if p.tokens.get(p.at) != Some(&Token::End) || !root_shape(&root) {
        return Err(Error::Unsupported);
    }
    Ok(Program {
        root,
        used: p.used,
        calendar: p.calendar,
    })
}
