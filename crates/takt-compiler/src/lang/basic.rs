//! BASIC structuré, traduit ligne à ligne en ST (une ligne BASIC = une ligne
//! ST, pour que les erreurs pointent la bonne ligne).
//!
//! ```basic
//! ' Commentaire
//! DIM compteur AS INT = 0
//! IF "Capteur" AND NOT "Defaut" THEN
//!     compteur = compteur + 1
//! ELSEIF "Reset" THEN
//!     compteur = 0
//! END IF
//! FOR i = 0 TO 7 STEP 1
//!     total = total + mesures[i]
//! NEXT i
//! SELECT CASE etape
//!     CASE 0, 1 : SET "Moteur"
//!     CASE ELSE : RESET "Moteur"
//! END SELECT
//! CALL tempo(IN := "Capteur", PT := T#2s)
//! ```
//!
//! `WHILE`, `DO … LOOP` et `GOTO` sont refusés comme en ST : leur nombre de
//! tours n'est pas borné.

use super::{Block, Generated, Hidden, Origin, Section};

pub const KEYWORDS: &[&str] = &[
    "IF", "THEN", "ELSE", "ELSEIF", "END", "ENDIF", "FOR", "TO", "STEP", "NEXT", "SELECT", "CASE", "DIM", "AS", "LET",
    "CALL", "SET", "RESET", "TOGGLE", "RETURN", "REM", "AND", "OR", "NOT", "XOR", "MOD", "TRUE", "FALSE", "WHILE",
    "WEND", "DO", "LOOP", "GOTO", "GOSUB",
];

/// Sépare le code d'un commentaire `'` (hors des noms entre guillemets).
fn split_comment(line: &str) -> (&str, Option<&str>) {
    let mut quoted = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '\'' if !quoted => return (&line[..i], Some(&line[i + 1..])),
            _ => {}
        }
    }
    let t = line.trim_start();
    if t.len() >= 3 && t[..3].eq_ignore_ascii_case("REM") && t[3..].chars().next().is_none_or(|c| c.is_whitespace()) {
        let start = line.len() - t.len();
        return (&line[..start], Some(&t[3..]));
    }
    (line, None)
}

/// Premier mot, en majuscules, et le reste.
fn head(t: &str) -> (String, &str) {
    let end = t.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(t.len());
    (t[..end].to_ascii_uppercase(), t[end..].trim_start())
}

/// Position du mot-clé `kw` (hors guillemets, entouré d'espaces).
fn find_kw(t: &str, kw: &str) -> Option<usize> {
    let upper = t.to_ascii_uppercase();
    let mut quoted = false;
    let bytes = upper.as_bytes();
    for (i, c) in upper.char_indices() {
        if c == '"' {
            quoted = !quoted;
        }
        if quoted || !upper[i..].starts_with(kw) {
            continue;
        }
        let before = i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
        let after = upper[i + kw.len()..].chars().next().is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'));
        if before && after {
            return Some(i);
        }
    }
    None
}

/// `x = expr` → `Some(("x", "expr"))` si la ligne est une affectation.
fn assignment(t: &str) -> Option<(&str, &str)> {
    let mut quoted = false;
    let mut depth = 0i32;
    for (i, c) in t.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '[' | '(' if !quoted => depth += 1,
            ']' | ')' if !quoted => depth -= 1,
            '=' if !quoted && depth == 0 => {
                let prev = t[..i].chars().last();
                if matches!(prev, Some('<') | Some('>') | Some(':')) {
                    return None;
                }
                let lhs = t[..i].trim();
                let valid = !lhs.is_empty()
                    && lhs.chars().all(|c| c.is_ascii_alphanumeric() || "_.[]#\" ".contains(c))
                    && (lhs.starts_with('"') || !lhs.contains(' '));
                return valid.then(|| (lhs, t[i + 1..].trim()));
            }
            _ => {}
        }
    }
    None
}

/// Une instruction simple (après THEN ou après `CASE x :`).
fn statement(t: &str) -> String {
    let t = t.trim();
    let (w, rest) = head(t);
    match w.as_str() {
        "" if t.is_empty() => String::new(),
        "LET" => statement(rest),
        "SET" => format!("{} := TRUE;", rest.trim()),
        "RESET" => format!("{} := FALSE;", rest.trim()),
        "TOGGLE" => format!("{0} := NOT {0};", rest.trim()),
        "CALL" => format!("{};", rest.trim().trim_end_matches(';')),
        "RETURN" | "END" if rest.is_empty() => "RETURN;".into(),
        "GOTO" | "GOSUB" => "GOTO;".into(),
        _ => match assignment(t) {
            Some((lhs, rhs)) => format!("{lhs} := {rhs};"),
            None => format!("{};", t.trim_end_matches(';')),
        },
    }
}

/// Condition d'un `ELSEIF … THEN`, sans le `THEN`.
fn before_then(t: &str) -> &str {
    match find_kw(t, "THEN") {
        Some(i) => t[..i].trim(),
        None => t.trim(),
    }
}

/// `1, 2, 5 TO 7` → `1, 2, 5..7`.
fn case_labels(t: &str) -> String {
    t.split(',')
        .map(|p| match find_kw(p, "TO") {
            Some(i) => format!("{}..{}", p[..i].trim(), p[i + 2..].trim()),
            None => p.trim().to_string(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn translate_line(code: &str, hidden: &mut Vec<Hidden>, block: &Block) -> String {
    let t = code.trim();
    let (w, rest) = head(t);
    match w.as_str() {
        "" if t.is_empty() => String::new(),
        "DIM" => {
            // DIM nom AS TYPE [= valeur]
            let (decl, init) = match rest.find('=') {
                Some(eq) => (&rest[..eq], rest[eq + 1..].trim()),
                None => (rest, ""),
            };
            match find_kw(decl, "AS") {
                Some(i) => {
                    for name in decl[..i].split(',') {
                        let name = name.trim();
                        if !name.is_empty() && !block.declares(name) {
                            hidden.push(Hidden {
                                section: Section::Static,
                                name: name.into(),
                                ty: decl[i + 2..].trim().into(),
                                init: init.into(),
                            });
                        }
                    }
                    String::new()
                }
                None => "DIM_sans_type := AS;".into(),
            }
        }
        "IF" => match find_kw(t, "THEN") {
            Some(i) => {
                let cond = t[2..i].trim();
                let after = t[i + 4..].trim();
                if after.is_empty() {
                    format!("IF {cond} THEN")
                } else {
                    match find_kw(after, "ELSE") {
                        Some(e) => format!(
                            "IF {cond} THEN {} ELSE {} END_IF;",
                            statement(&after[..e]),
                            statement(&after[e + 4..])
                        ),
                        None => format!("IF {cond} THEN {} END_IF;", statement(after)),
                    }
                }
            }
            None => format!("IF {} THEN", t[2..].trim()),
        },
        "ELSEIF" => format!("ELSIF {} THEN", before_then(rest)),
        "ELSE" => {
            let (w2, rest2) = head(rest);
            if w2 == "IF" { format!("ELSIF {} THEN", before_then(rest2)) } else { "ELSE".into() }
        }
        "ENDIF" => "END_IF;".into(),
        "END" => {
            let (w2, _) = head(rest);
            match w2.as_str() {
                "IF" => "END_IF;".into(),
                "SELECT" => "END_CASE;".into(),
                "" => "RETURN;".into(),
                _ => format!("END_{w2};"),
            }
        }
        "FOR" => {
            // FOR v = a TO b [STEP s]
            let (var, range) = match assignment(rest) {
                Some((v, r)) => (v, r),
                None => return format!("FOR {rest} DO"),
            };
            let (from, to_part) = match find_kw(range, "TO") {
                Some(i) => (range[..i].trim(), range[i + 2..].trim()),
                None => (range, ""),
            };
            match find_kw(to_part, "STEP") {
                Some(s) => format!("FOR {var} := {from} TO {} BY {} DO", to_part[..s].trim(), to_part[s + 4..].trim()),
                None => format!("FOR {var} := {from} TO {to_part} DO"),
            }
        }
        "NEXT" => "END_FOR;".into(),
        "SELECT" => {
            let (_, expr) = head(rest);
            format!("CASE {} OF", expr.trim())
        }
        "CASE" => {
            let (w2, _) = head(rest);
            let (labels, stmt) = match rest.find(':') {
                Some(i) if w2 != "ELSE" => (&rest[..i], &rest[i + 1..]),
                _ => (rest, ""),
            };
            if w2 == "ELSE" {
                let after = rest[4..].trim().trim_start_matches(':');
                format!("ELSE {}", statement(after))
            } else {
                format!("{}: {}", case_labels(labels), statement(stmt))
            }
        }
        "WHILE" => format!("WHILE {} DO", rest),
        "WEND" => "END_WHILE;".into(),
        "DO" => "WHILE TRUE DO".into(),
        "LOOP" => "END_WHILE;".into(),
        _ => statement(t),
    }
}

/// Les `DIM` deviennent des variables ajoutées à l'interface du bloc.
pub fn translate(block: &Block, out: &mut Generated, hidden: &mut Vec<Hidden>) {
    let mut lines = Vec::new();
    for (i, raw) in block.source.lines().enumerate() {
        let (code, comment) = split_comment(raw);
        let indent: String = raw.chars().take_while(|c| c.is_whitespace()).collect();
        let st = translate_line(code, hidden, block);
        let comment = comment.map(|c| format!(" // {}", c.trim())).unwrap_or_default();
        let line =
            if st.is_empty() { format!("{indent}{}", comment.trim_start()) } else { format!("{indent}{st}{comment}") };
        lines.push((line, Origin::Line(i as u32 + 1)));
    }
    out.lines.extend(lines);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(line: &str) -> String {
        translate_line(line, &mut Vec::new(), &Block::empty("t"))
    }

    #[test]
    fn traduction_ligne_a_ligne() {
        assert_eq!(one("x = x + 1"), "x := x + 1;");
        assert_eq!(one("LET \"Moteur\" = a AND b"), "\"Moteur\" := a AND b;");
        assert_eq!(one("IF a <= 3 THEN"), "IF a <= 3 THEN");
        assert_eq!(one("IF a = 3 THEN SET y ELSE RESET y"), "IF a = 3 THEN y := TRUE; ELSE y := FALSE; END_IF;");
        assert_eq!(one("ELSEIF b THEN"), "ELSIF b THEN");
        assert_eq!(one("ELSE IF b THEN"), "ELSIF b THEN");
        assert_eq!(one("END IF"), "END_IF;");
        assert_eq!(one("FOR i = 0 TO 7 STEP 2"), "FOR i := 0 TO 7 BY 2 DO");
        assert_eq!(one("NEXT i"), "END_FOR;");
        assert_eq!(one("SELECT CASE etape"), "CASE etape OF");
        assert_eq!(one("CASE 1, 3 TO 5 : x = 2"), "1, 3..5: x := 2;");
        assert_eq!(one("CASE ELSE : x = 0"), "ELSE x := 0;");
        assert_eq!(one("END SELECT"), "END_CASE;");
        assert_eq!(one("CALL t1(IN := a, PT := T#1s)"), "t1(IN := a, PT := T#1s);");
        assert_eq!(one("WHILE a"), "WHILE a DO");
        assert_eq!(split_comment("x = 1 ' compteur"), ("x = 1 ", Some(" compteur")));
        assert_eq!(split_comment("REM tout"), ("", Some(" tout")));
        let mut h = Vec::new();
        translate_line("DIM a, b AS INT = 4", &mut h, &Block::empty("t"));
        assert_eq!(h.len(), 2);
        assert_eq!((h[1].name.as_str(), h[1].ty.as_str(), h[1].init.as_str()), ("b", "INT", "4"));
    }
}
