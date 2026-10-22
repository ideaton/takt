//! Carte « Instructions » : catalogue cliquable, comme la task card de TIA.
//! Un bloc standard inséré est aussi déclaré en multi-instance dans `VAR`.

pub struct Instr {
    pub label: &'static str,
    pub help: &'static str,
    /// Texte inséré ; `{n}` est remplacé par le nom de l'instance créée.
    pub snippet: &'static str,
    /// Type de l'instance à déclarer, et préfixe de son nom.
    pub instance: Option<(&'static str, &'static str)>,
}

pub struct Group {
    pub name: &'static str,
    pub items: &'static [Instr],
}

const fn i(label: &'static str, help: &'static str, snippet: &'static str) -> Instr {
    Instr { label, help, snippet, instance: None }
}

const fn fb(
    label: &'static str,
    help: &'static str,
    snippet: &'static str,
    ty: &'static str,
    prefix: &'static str,
) -> Instr {
    Instr { label, help, snippet, instance: Some((ty, prefix)) }
}

pub const CATALOG: &[Group] = &[
    Group {
        name: "Opérations logiques sur bits",
        items: &[
            i(":=  Affectation", "Écrit le résultat d'une expression", "#var := #a AND NOT #b;\n"),
            fb(
                "R_TRIG  Front montant",
                "Q vaut TRUE pendant un cycle quand CLK passe à TRUE",
                "#{n}(CLK := );\n",
                "R_TRIG",
                "Front",
            ),
            fb(
                "F_TRIG  Front descendant",
                "Q vaut TRUE pendant un cycle quand CLK passe à FALSE",
                "#{n}(CLK := );\n",
                "F_TRIG",
                "FrontDesc",
            ),
            fb(
                "SR  Bascule mise à 1 prioritaire",
                "Q1 := S1 OR (NOT R AND Q1)",
                "#{n}(S1 := , R := );\n",
                "SR",
                "Bascule",
            ),
            fb(
                "RS  Bascule mise à 0 prioritaire",
                "Q1 := NOT R1 AND (S OR Q1)",
                "#{n}(S := , R1 := );\n",
                "RS",
                "Bascule",
            ),
        ],
    },
    Group {
        name: "Temporisations",
        items: &[
            fb(
                "TON  Retard à la montée",
                "Q passe à TRUE quand IN est resté TRUE pendant PT",
                "#{n}(IN := , PT := T#1s);\n",
                "TON",
                "Tempo",
            ),
            fb(
                "TOF  Retard à la retombée",
                "Q reste TRUE pendant PT après la retombée de IN",
                "#{n}(IN := , PT := T#1s);\n",
                "TOF",
                "Tempo",
            ),
            fb(
                "TP  Impulsion",
                "Q vaut TRUE pendant PT sur front montant de IN",
                "#{n}(IN := , PT := T#500ms);\n",
                "TP",
                "Impulsion",
            ),
        ],
    },
    Group {
        name: "Comptage",
        items: &[
            fb(
                "CTU  Compteur",
                "CV augmente sur chaque front de CU ; Q quand CV >= PV",
                "#{n}(CU := , R := , PV := 10);\n",
                "CTU",
                "Compteur",
            ),
            fb(
                "CTD  Décompteur",
                "CV diminue sur chaque front de CD ; Q quand CV <= 0",
                "#{n}(CD := , LD := , PV := 10);\n",
                "CTD",
                "Decompteur",
            ),
        ],
    },
    Group {
        name: "Contrôle du programme",
        items: &[
            i("IF … THEN", "Exécution conditionnelle", "IF  THEN\n    \nEND_IF;\n"),
            i("IF … ELSE", "Deux branches", "IF  THEN\n    \nELSE\n    \nEND_IF;\n"),
            i(
                "CASE",
                "Choix selon une valeur entière",
                "CASE #etape OF\n    0:\n        \n    1:\n        \nELSE\n    \nEND_CASE;\n",
            ),
            i(
                "FOR  (bornes constantes)",
                "Boucle au nombre de tours connu : le temps de cycle reste garanti",
                "FOR #i := 0 TO 9 DO\n    \nEND_FOR;\n",
            ),
            i("RETURN", "Quitte le bloc", "RETURN;\n"),
        ],
    },
    Group {
        name: "Comparaison et mathématiques",
        items: &[
            i("ABS", "Valeur absolue", "ABS()"),
            i("MIN / MAX", "Plus petite / plus grande valeur", "MIN(, )"),
            i("LIMIT", "Borne IN entre MN et MX", "LIMIT(MN := , IN := , MX := )"),
            i("SEL", "IN0 si G = FALSE, IN1 sinon", "SEL(G := , IN0 := , IN1 := )"),
            i("MOD", "Reste de la division entière", " MOD "),
        ],
    },
    Group {
        name: "Conversion",
        items: &[
            i("NORM_X", "Normalise VALUE entre MIN et MAX vers 0.0 … 1.0", "NORM_X(MIN := 0, VALUE := , MAX := 27648)"),
            i("SCALE_X", "Met à l'échelle 0.0 … 1.0 vers MIN … MAX", "SCALE_X(MIN := 0.0, VALUE := , MAX := 100.0)"),
            i("INT_TO_REAL", "Entier vers réel", "INT_TO_REAL()"),
            i("REAL_TO_INT", "Réel vers entier, arrondi", "REAL_TO_INT()"),
            i("TRUNC", "Réel vers DINT, partie entière", "TRUNC()"),
            i("TIME_TO_DINT", "Durée en ms", "TIME_TO_DINT()"),
        ],
    },
];

/// Ajoute `nom : type;` dans la première section `VAR` du bloc (ou en crée une).
pub fn declare_instance(source: &mut String, ty: &str, prefix: &str) -> String {
    let upper = source.to_ascii_uppercase();
    let mut k = 1;
    let name = loop {
        let n = format!("{prefix}_{k}");
        if !upper.contains(&n.to_ascii_uppercase()) {
            break n;
        }
        k += 1;
    };
    let decl = format!("    {name} : {ty};\n");
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        if line.trim().eq_ignore_ascii_case("VAR") {
            source.insert_str(offset + line.len(), &decl);
            return name;
        }
        offset += line.len();
    }
    // Pas de section VAR : on en crée une après la ligne d'en-tête
    let header_end = source.find('\n').map(|p| p + 1).unwrap_or(source.len());
    source.insert_str(header_end, &format!("VAR\n{decl}END_VAR\n"));
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declaration_multi_instance() {
        let mut s = "ORGANIZATION_BLOCK Main\nVAR\n    x : INT;\nEND_VAR\nBEGIN\nEND_ORGANIZATION_BLOCK\n".to_string();
        assert_eq!(declare_instance(&mut s, "TON", "Tempo"), "Tempo_1");
        assert_eq!(declare_instance(&mut s, "TON", "Tempo"), "Tempo_2");
        assert!(s.contains("VAR\n    Tempo_2 : TON;\n    Tempo_1 : TON;\n    x : INT;"));
        let mut s = "FUNCTION_BLOCK F\nBEGIN\nEND_FUNCTION_BLOCK\n".to_string();
        declare_instance(&mut s, "CTU", "Compteur");
        assert!(s.starts_with("FUNCTION_BLOCK F\nVAR\n    Compteur_1 : CTU;\nEND_VAR\nBEGIN"));
    }
}
