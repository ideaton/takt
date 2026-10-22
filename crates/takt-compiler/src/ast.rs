//! Arbre syntaxique du Structured Text.

use crate::lexer::Pos;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PouKind {
    /// `PROGRAM` ou `ORGANIZATION_BLOCK` : exécuté à chaque cycle.
    Program,
    FunctionBlock,
    Function,
}

#[derive(Clone, Debug)]
pub struct Pou {
    pub kind: PouKind,
    pub name: String,
    pub pos: Pos,
    /// Type de retour d'une `FUNCTION`.
    pub ret: Option<TypeRef>,
    pub vars: Vec<VarDecl>,
    pub body: Vec<Stmt>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VarSection {
    Input,
    Output,
    InOut,
    Static,
    Temp,
    Constant,
}

#[derive(Clone, Debug)]
pub struct VarDecl {
    pub name: String,
    pub pos: Pos,
    pub section: VarSection,
    pub ty: TypeRef,
    pub init: Option<Expr>,
}

#[derive(Clone, Debug)]
pub enum TypeRef {
    Named(String, Pos),
    Array { lo: Expr, hi: Expr, elem: Box<TypeRef> },
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Assign {
        target: Expr,
        value: Expr,
        pos: Pos,
    },
    If {
        branches: Vec<(Expr, Vec<Stmt>)>,
        otherwise: Vec<Stmt>,
        pos: Pos,
    },
    Case {
        selector: Expr,
        arms: Vec<CaseArm>,
        otherwise: Vec<Stmt>,
        pos: Pos,
    },
    For {
        var: String,
        from: Expr,
        to: Expr,
        by: Option<Expr>,
        body: Vec<Stmt>,
        pos: Pos,
    },
    /// Appel d'instance de FB : `t1(IN := x, PT := T#1s);`
    Call {
        callee: Expr,
        args: Vec<Arg>,
        pos: Pos,
    },
    Return {
        pos: Pos,
    },
    /// `WHILE` / `REPEAT` : analysés pour donner un message clair, puis refusés.
    Unbounded {
        keyword: &'static str,
        pos: Pos,
    },
    Empty,
}

#[derive(Clone, Debug)]
pub struct CaseArm {
    pub labels: Vec<CaseLabel>,
    pub body: Vec<Stmt>,
}

#[derive(Clone, Debug)]
pub enum CaseLabel {
    Value(Expr),
    Range(Expr, Expr),
}

#[derive(Clone, Debug)]
pub struct Arg {
    /// `None` pour un argument positionnel.
    pub name: Option<String>,
    /// Vrai pour `NOM => variable` (sortie).
    pub output: bool,
    pub value: Expr,
    pub pos: Pos,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Or,
    Xor,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Clone, Debug)]
pub enum Expr {
    Int(i64, Pos),
    Real(f64, Pos),
    Bool(bool, Pos),
    Time(i64, Pos),
    /// Nom simple. `scope` dit si la syntaxe imposait local (`#x`) ou global (`"x"`).
    Name {
        name: String,
        scope: NameScope,
        pos: Pos,
    },
    Field {
        base: Box<Expr>,
        field: String,
        pos: Pos,
    },
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
        pos: Pos,
    },
    Unary {
        op: UnOp,
        e: Box<Expr>,
        pos: Pos,
    },
    Binary {
        op: BinOp,
        l: Box<Expr>,
        r: Box<Expr>,
        pos: Pos,
    },
    Call {
        name: String,
        args: Vec<Arg>,
        pos: Pos,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameScope {
    Any,
    Local,
    Global,
}

impl Expr {
    pub fn pos(&self) -> Pos {
        match self {
            Expr::Int(_, p) | Expr::Real(_, p) | Expr::Bool(_, p) | Expr::Time(_, p) => *p,
            Expr::Name { pos, .. }
            | Expr::Field { pos, .. }
            | Expr::Index { pos, .. }
            | Expr::Unary { pos, .. }
            | Expr::Binary { pos, .. }
            | Expr::Call { pos, .. } => *pos,
        }
    }
}
