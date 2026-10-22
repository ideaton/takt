//! Analyse syntaxique : jetons → [`Pou`].

use crate::ast::*;
use crate::lexer::{Pos, Tok, Token, lex};

#[derive(Debug, Clone)]
pub struct ParseError {
    pub pos: Pos,
    pub msg: String,
}

type R<T> = Result<T, ParseError>;

/// Analyse une source contenant un ou plusieurs blocs.
pub fn parse(src: &str) -> R<Vec<Pou>> {
    let toks = lex(src).map_err(|e| ParseError { pos: e.pos, msg: e.msg })?;
    let mut p = Parser { toks, i: 0 };
    let mut pous = Vec::new();
    while p.peek() != &Tok::Eof {
        pous.push(p.pou()?);
    }
    if pous.is_empty() {
        return Err(ParseError { pos: Pos { line: 1, col: 1 }, msg: "bloc vide".into() });
    }
    Ok(pous)
}

/// Analyse un type seul (colonne « Type » de la table des variables API).
pub fn parse_type(src: &str) -> R<TypeRef> {
    let toks = lex(src).map_err(|e| ParseError { pos: e.pos, msg: e.msg })?;
    let mut p = Parser { toks, i: 0 };
    let t = p.type_ref()?;
    if p.peek() != &Tok::Eof {
        return p.err("fin du type attendue");
    }
    Ok(t)
}

struct Parser {
    toks: Vec<Token>,
    i: usize,
}

const STMT_END: &[&str] = &[
    "END_IF",
    "ELSIF",
    "ELSE",
    "END_CASE",
    "END_FOR",
    "END_WHILE",
    "END_REPEAT",
    "UNTIL",
    "END_PROGRAM",
    "END_ORGANIZATION_BLOCK",
    "END_FUNCTION_BLOCK",
    "END_FUNCTION",
];

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.i].tok
    }

    fn peek_at(&self, n: usize) -> &Tok {
        &self.toks[(self.i + n).min(self.toks.len() - 1)].tok
    }

    fn pos(&self) -> Pos {
        self.toks[self.i].pos
    }

    fn bump(&mut self) -> Token {
        let t = self.toks[self.i].clone();
        if self.i < self.toks.len() - 1 {
            self.i += 1;
        }
        t
    }

    fn err<T>(&self, msg: impl Into<String>) -> R<T> {
        Err(ParseError { pos: self.pos(), msg: msg.into() })
    }

    fn at_kw(&self, kw: &str) -> bool {
        self.peek().is_kw(kw)
    }

    fn eat_kw(&mut self, kw: &str) -> bool {
        if self.at_kw(kw) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect_kw(&mut self, kw: &str) -> R<()> {
        if self.eat_kw(kw) {
            Ok(())
        } else {
            self.err(format!("« {kw} » attendu, trouvé {}", describe(self.peek())))
        }
    }

    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == t {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, t: &Tok, what: &str) -> R<()> {
        if self.eat(t) { Ok(()) } else { self.err(format!("« {what} » attendu, trouvé {}", describe(self.peek()))) }
    }

    fn ident(&mut self) -> R<String> {
        match self.peek().clone() {
            Tok::Ident { name, .. } => {
                self.bump();
                Ok(name)
            }
            Tok::Quoted(name) => {
                self.bump();
                Ok(name)
            }
            t => self.err(format!("nom attendu, trouvé {}", describe(&t))),
        }
    }

    fn pou(&mut self) -> R<Pou> {
        let pos = self.pos();
        let (kind, end) = if self.eat_kw("PROGRAM") {
            (PouKind::Program, "END_PROGRAM")
        } else if self.eat_kw("ORGANIZATION_BLOCK") {
            (PouKind::Program, "END_ORGANIZATION_BLOCK")
        } else if self.eat_kw("FUNCTION_BLOCK") {
            (PouKind::FunctionBlock, "END_FUNCTION_BLOCK")
        } else if self.eat_kw("FUNCTION") {
            (PouKind::Function, "END_FUNCTION")
        } else {
            return self.err("PROGRAM, ORGANIZATION_BLOCK, FUNCTION_BLOCK ou FUNCTION attendu");
        };
        let name = self.ident()?;
        let mut ret = None;
        if kind == PouKind::Function && self.eat(&Tok::Colon) {
            if self.eat_kw("VOID") {
            } else {
                ret = Some(self.type_ref()?);
            }
        }
        let mut vars = Vec::new();
        loop {
            let section = if self.eat_kw("VAR_INPUT") {
                VarSection::Input
            } else if self.eat_kw("VAR_OUTPUT") {
                VarSection::Output
            } else if self.eat_kw("VAR_IN_OUT") {
                VarSection::InOut
            } else if self.eat_kw("VAR_TEMP") {
                VarSection::Temp
            } else if self.eat_kw("VAR_STAT") {
                VarSection::Static
            } else if self.eat_kw("VAR") {
                if self.eat_kw("CONSTANT") { VarSection::Constant } else { VarSection::Static }
            } else {
                break;
            };
            while self.eat_kw("RETAIN") || self.eat_kw("NON_RETAIN") {}
            if section == VarSection::InOut {
                return self.err("VAR_IN_OUT n'est pas encore supporté");
            }
            while !self.at_kw("END_VAR") {
                vars.extend(self.var_decl(section)?);
            }
            self.bump();
        }
        self.eat_kw("BEGIN");
        let body = self.stmts()?;
        self.expect_kw(end)?;
        Ok(Pou { kind, name, pos, ret, vars, body })
    }

    fn var_decl(&mut self, section: VarSection) -> R<Vec<VarDecl>> {
        let mut names = vec![(self.ident()?, self.toks[self.i - 1].pos)];
        while self.eat(&Tok::Comma) {
            names.push((self.ident()?, self.toks[self.i - 1].pos));
        }
        self.expect(&Tok::Colon, ":")?;
        let ty = self.type_ref()?;
        let init = if self.eat(&Tok::Assign) { Some(self.expr()?) } else { None };
        self.expect(&Tok::Semi, ";")?;
        Ok(names
            .into_iter()
            .map(|(name, pos)| VarDecl { name, pos, section, ty: ty.clone(), init: init.clone() })
            .collect())
    }

    fn type_ref(&mut self) -> R<TypeRef> {
        if self.eat_kw("ARRAY") {
            self.expect(&Tok::LBracket, "[")?;
            let lo = self.expr()?;
            self.expect(&Tok::DotDot, "..")?;
            let hi = self.expr()?;
            self.expect(&Tok::RBracket, "]")?;
            self.expect_kw("OF")?;
            let elem = self.type_ref()?;
            return Ok(TypeRef::Array { lo, hi, elem: Box::new(elem) });
        }
        let pos = self.pos();
        Ok(TypeRef::Named(self.ident()?, pos))
    }

    fn at_stmt_end(&self) -> bool {
        STMT_END.iter().any(|kw| self.at_kw(kw)) || self.peek() == &Tok::Eof
    }

    fn stmts(&mut self) -> R<Vec<Stmt>> {
        let mut v = Vec::new();
        while !self.at_stmt_end() {
            v.push(self.stmt()?);
        }
        Ok(v)
    }

    /// Dans un CASE : vrai si on est au début d'un nouveau libellé (`3:`, `1..5:`, `2, 4:`).
    fn at_case_label(&self) -> bool {
        let mut n = 0;
        if self.peek_at(0) == &Tok::Minus {
            n = 1;
        }
        matches!(self.peek_at(n), Tok::Int(_)) && matches!(self.peek_at(n + 1), Tok::Colon | Tok::Comma | Tok::DotDot)
    }

    fn stmt(&mut self) -> R<Stmt> {
        let pos = self.pos();
        if self.eat(&Tok::Semi) {
            return Ok(Stmt::Empty);
        }
        if self.eat_kw("IF") {
            let mut branches = Vec::new();
            let cond = self.expr()?;
            self.expect_kw("THEN")?;
            branches.push((cond, self.stmts()?));
            let mut otherwise = Vec::new();
            loop {
                if self.eat_kw("ELSIF") {
                    let c = self.expr()?;
                    self.expect_kw("THEN")?;
                    branches.push((c, self.stmts()?));
                } else if self.eat_kw("ELSE") {
                    otherwise = self.stmts()?;
                } else {
                    break;
                }
            }
            self.expect_kw("END_IF")?;
            self.eat(&Tok::Semi);
            return Ok(Stmt::If { branches, otherwise, pos });
        }
        if self.eat_kw("CASE") {
            let selector = self.expr()?;
            self.expect_kw("OF")?;
            let mut arms = Vec::new();
            let mut otherwise = Vec::new();
            loop {
                if self.eat_kw("ELSE") {
                    otherwise = self.stmts()?;
                    break;
                }
                if !self.at_case_label() {
                    break;
                }
                let mut labels = Vec::new();
                loop {
                    let a = self.unary()?;
                    if self.eat(&Tok::DotDot) {
                        labels.push(CaseLabel::Range(a, self.unary()?));
                    } else {
                        labels.push(CaseLabel::Value(a));
                    }
                    if !self.eat(&Tok::Comma) {
                        break;
                    }
                }
                self.expect(&Tok::Colon, ":")?;
                let mut body = Vec::new();
                while !self.at_stmt_end() && !self.at_case_label() {
                    body.push(self.stmt()?);
                }
                arms.push(CaseArm { labels, body });
            }
            self.expect_kw("END_CASE")?;
            self.eat(&Tok::Semi);
            return Ok(Stmt::Case { selector, arms, otherwise, pos });
        }
        if self.eat_kw("FOR") {
            let var = self.ident()?;
            self.expect(&Tok::Assign, ":=")?;
            let from = self.expr()?;
            self.expect_kw("TO")?;
            let to = self.expr()?;
            let by = if self.eat_kw("BY") { Some(self.expr()?) } else { None };
            self.expect_kw("DO")?;
            let body = self.stmts()?;
            self.expect_kw("END_FOR")?;
            self.eat(&Tok::Semi);
            return Ok(Stmt::For { var, from, to, by, body, pos });
        }
        if self.eat_kw("WHILE") {
            self.expr()?;
            self.expect_kw("DO")?;
            self.stmts()?;
            self.expect_kw("END_WHILE")?;
            self.eat(&Tok::Semi);
            return Ok(Stmt::Unbounded { keyword: "WHILE", pos });
        }
        if self.eat_kw("REPEAT") {
            self.stmts()?;
            self.expect_kw("UNTIL")?;
            self.expr()?;
            self.expect_kw("END_REPEAT")?;
            self.eat(&Tok::Semi);
            return Ok(Stmt::Unbounded { keyword: "REPEAT", pos });
        }
        if self.eat_kw("RETURN") {
            self.expect(&Tok::Semi, ";")?;
            return Ok(Stmt::Return { pos });
        }
        if self.at_kw("EXIT") || self.at_kw("CONTINUE") || self.at_kw("GOTO") {
            return self.err("EXIT, CONTINUE et GOTO ne sont pas supportés");
        }
        let target = self.postfix()?;
        if let Expr::Call { name, args, pos } = target {
            self.expect(&Tok::Semi, ";")?;
            return Ok(Stmt::Call { callee: Expr::Name { name, scope: NameScope::Any, pos }, args, pos });
        }
        if !self.eat(&Tok::Assign) {
            return self.err(format!("« := » attendu, trouvé {}", describe(self.peek())));
        }
        let value = self.expr()?;
        self.expect(&Tok::Semi, ";")?;
        Ok(Stmt::Assign { target, value, pos })
    }

    pub fn expr(&mut self) -> R<Expr> {
        self.binary(0)
    }

    fn binop(&self, level: usize) -> Option<BinOp> {
        let t = self.peek();
        match level {
            0 if t.is_kw("OR") => Some(BinOp::Or),
            1 if t.is_kw("XOR") => Some(BinOp::Xor),
            2 if t.is_kw("AND") || t == &Tok::Amp => Some(BinOp::And),
            3 => match t {
                Tok::Eq => Some(BinOp::Eq),
                Tok::Ne => Some(BinOp::Ne),
                Tok::Lt => Some(BinOp::Lt),
                Tok::Le => Some(BinOp::Le),
                Tok::Gt => Some(BinOp::Gt),
                Tok::Ge => Some(BinOp::Ge),
                _ => None,
            },
            4 => match t {
                Tok::Plus => Some(BinOp::Add),
                Tok::Minus => Some(BinOp::Sub),
                _ => None,
            },
            5 => match t {
                Tok::Star => Some(BinOp::Mul),
                Tok::Slash => Some(BinOp::Div),
                _ if t.is_kw("MOD") => Some(BinOp::Mod),
                _ => None,
            },
            _ => None,
        }
    }

    fn binary(&mut self, level: usize) -> R<Expr> {
        if level > 5 {
            return self.unary();
        }
        let mut l = self.binary(level + 1)?;
        while let Some(op) = self.binop(level) {
            let pos = self.pos();
            self.bump();
            let r = self.binary(level + 1)?;
            l = Expr::Binary { op, l: Box::new(l), r: Box::new(r), pos };
        }
        Ok(l)
    }

    fn unary(&mut self) -> R<Expr> {
        let pos = self.pos();
        if self.eat(&Tok::Minus) {
            let e = self.unary()?;
            return Ok(match e {
                Expr::Int(v, _) => Expr::Int(-v, pos),
                Expr::Real(v, _) => Expr::Real(-v, pos),
                Expr::Time(v, _) => Expr::Time(-v, pos),
                e => Expr::Unary { op: UnOp::Neg, e: Box::new(e), pos },
            });
        }
        if self.eat(&Tok::Plus) {
            return self.unary();
        }
        if self.eat_kw("NOT") {
            let e = self.unary()?;
            return Ok(Expr::Unary { op: UnOp::Not, e: Box::new(e), pos });
        }
        self.postfix()
    }

    fn postfix(&mut self) -> R<Expr> {
        let pos = self.pos();
        let mut e = match self.bump().tok {
            Tok::Int(v) => return Ok(Expr::Int(v, pos)),
            Tok::Real(v) => return Ok(Expr::Real(v, pos)),
            Tok::Time(v) => return Ok(Expr::Time(v, pos)),
            Tok::LParen => {
                let e = self.expr()?;
                self.expect(&Tok::RParen, ")")?;
                return Ok(e);
            }
            Tok::Ident { name, local } => {
                if !local && name.eq_ignore_ascii_case("TRUE") {
                    return Ok(Expr::Bool(true, pos));
                }
                if !local && name.eq_ignore_ascii_case("FALSE") {
                    return Ok(Expr::Bool(false, pos));
                }
                if !local && self.peek() == &Tok::LParen {
                    self.bump();
                    let args = self.args()?;
                    return Ok(Expr::Call { name, args, pos });
                }
                Expr::Name { name, scope: if local { NameScope::Local } else { NameScope::Any }, pos }
            }
            Tok::Quoted(name) => Expr::Name { name, scope: NameScope::Global, pos },
            t => {
                self.i -= 1;
                return self.err(format!("expression attendue, trouvé {}", describe(&t)));
            }
        };
        loop {
            let pos = self.pos();
            if self.eat(&Tok::Dot) {
                let field = self.ident()?;
                e = Expr::Field { base: Box::new(e), field, pos };
            } else if self.eat(&Tok::LBracket) {
                let index = self.expr()?;
                self.expect(&Tok::RBracket, "]")?;
                e = Expr::Index { base: Box::new(e), index: Box::new(index), pos };
            } else if self.peek() == &Tok::LParen {
                // Appel d'instance : `#t1(IN := …)`
                if let Expr::Name { name, .. } = &e {
                    let name = name.clone();
                    self.bump();
                    let args = self.args()?;
                    return Ok(Expr::Call { name, args, pos: e.pos() });
                }
                return self.err("appel invalide");
            } else {
                return Ok(e);
            }
        }
    }

    fn args(&mut self) -> R<Vec<Arg>> {
        let mut args = Vec::new();
        if self.eat(&Tok::RParen) {
            return Ok(args);
        }
        loop {
            let pos = self.pos();
            let named =
                matches!(self.peek(), Tok::Ident { .. }) && matches!(self.peek_at(1), Tok::Assign | Tok::OutAssign);
            if named {
                let name = self.ident()?;
                let output = self.bump().tok == Tok::OutAssign;
                let value = self.expr()?;
                args.push(Arg { name: Some(name), output, value, pos });
            } else {
                args.push(Arg { name: None, output: false, value: self.expr()?, pos });
            }
            if self.eat(&Tok::RParen) {
                return Ok(args);
            }
            self.expect(&Tok::Comma, ",")?;
        }
    }
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Ident { name, local: true } => format!("« #{name} »"),
        Tok::Ident { name, .. } => format!("« {name} »"),
        Tok::Quoted(n) => format!("« \"{n}\" »"),
        Tok::Int(v) => format!("« {v} »"),
        Tok::Real(v) => format!("« {v} »"),
        Tok::Time(v) => format!("« T#{v}ms »"),
        Tok::Eof => "la fin du bloc".into(),
        other => format!("« {} »", symbol(other)),
    }
}

fn symbol(t: &Tok) -> &'static str {
    match t {
        Tok::Assign => ":=",
        Tok::OutAssign => "=>",
        Tok::Semi => ";",
        Tok::Colon => ":",
        Tok::Comma => ",",
        Tok::Dot => ".",
        Tok::DotDot => "..",
        Tok::LParen => "(",
        Tok::RParen => ")",
        Tok::LBracket => "[",
        Tok::RBracket => "]",
        Tok::Plus => "+",
        Tok::Minus => "-",
        Tok::Star => "*",
        Tok::Slash => "/",
        Tok::Eq => "=",
        Tok::Ne => "<>",
        Tok::Lt => "<",
        Tok::Le => "<=",
        Tok::Gt => ">",
        Tok::Ge => ">=",
        Tok::Amp => "&",
        _ => "?",
    }
}
