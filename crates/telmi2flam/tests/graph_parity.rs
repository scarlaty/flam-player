//! Parité avec Python de la conversion du graphe `nodes.json` vers `script/nodes.lua` et de
//! `main.lua` : la table `tests/data/graph_cases.json` est produite par
//! `tests/data/gen_graph_cases.py` (conversion complète par `tools/telmi2flam/telmi2flam.py`).
//!
//! Pour chaque cas, le test rejoue les étapes de `_convert` qui touchent au graphe, dans
//! l'ordre de Python : `notes.json`, `stages` / `inventory`, stages, actions, inventaire,
//! assets manquants (`missing_asset`, simulé ici : il relève de la PR des assets),
//! `startAction`, puis `nodes.lua`. Les noms d'assets et l'en-tête (titre, couverture) sont
//! ceux que Python a passés à `lua_repr`. Les appels au trait `Assets` sont contrôlés (nom égal
//! au champ de `nodes.json`, ordre de Python, liste égale aux appels relevés chez Python dans
//! la colonne `calls`) ; les cas marqués `divergence` (`json_limit`)
//! vérifient l'écart assumé du Rust au lieu de la parité.

use std::collections::HashMap;

use telmi2flam::error::ConvError;
use telmi2flam::graph::{
    convert_graph, load_notes, nodes_lua, Assets, Header, Sources, Start, MAIN_LUA,
};
use telmi2flam::json::{load_json, Value};
use telmi2flam::text::Replaced;

fn table() -> Value {
    load_json(include_bytes!("data/graph_cases.json"), "graph_cases.json").unwrap()
}

fn field<'a>(v: &'a Value, key: &str) -> &'a Value {
    v.get(key).unwrap_or_else(|| panic!("champ {key} absent"))
}

fn str_of(v: &Value) -> &str {
    match v {
        Value::Str(s) => s,
        other => panic!("chaine attendue : {other:?}"),
    }
}

fn opt_str(v: Option<&Value>) -> Option<String> {
    match v {
        None | Some(Value::Null) => None,
        Some(v) => Some(str_of(v).to_owned()),
    }
}

fn list(v: &Value) -> &[Value] {
    match v {
        Value::List(items) => items,
        other => panic!("liste attendue : {other:?}"),
    }
}

/// Octets bruts d'un fichier du cas (`{"text": …}` ou `{"hex": …}`), `None` si absent.
fn raw(v: &Value) -> Option<Vec<u8>> {
    if matches!(v, Value::Null) {
        return None;
    }
    if let Some(t) = v.get("text") {
        return Some(str_of(t).as_bytes().to_vec());
    }
    let h = str_of(field(v, "hex"));
    Some(
        (0..h.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap())
            .collect(),
    )
}

/// Même représentation que `digest` du générateur.
fn digest(out: &[u8], keep: usize) -> String {
    if out.len() <= keep {
        return String::from_utf8(out.to_vec()).unwrap();
    }
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for &b in out {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01B3);
    }
    format!("fnv1a64:{h:016x}:{}", out.len())
}

/// Ordre des appels de Python : (0 stages / 1 inventaire, position, 0 image / 1 audio).
type OrderKey = (u8, usize, u8);

/// Noms d'assets rendus par Python, par champ (`stage s0.image`, `inventory[0].image`…), et
/// contrôle des appels reçus : nom passé, type d'appel et ordre.
struct Table<'a> {
    py: HashMap<String, Option<String>>,
    nodes: &'a Value,
    /// `what` de chaque appel, dans l'ordre reçu.
    calls: Vec<String>,
    /// Écarts constatés (nom, type d'appel, ordre).
    problems: Vec<String>,
    last: Option<OrderKey>,
}

impl<'a> Table<'a> {
    fn new(py: HashMap<String, Option<String>>, nodes: &'a Value) -> Self {
        Table {
            py,
            nodes,
            calls: Vec::new(),
            problems: Vec::new(),
            last: None,
        }
    }

    /// Clé d'ordre de Python (stages, image puis audio, puis inventaire) et valeur du champ
    /// dans `nodes.json`, pour un `what` donné.
    fn locate(&self, what: &str) -> Option<(OrderKey, Option<&'a Value>)> {
        let (head, field) = what.rsplit_once('.')?;
        let sub = match field {
            "image" => 0,
            "audio" => 1,
            _ => return None,
        };
        if let Some(sid) = head.strip_prefix("stage ") {
            let Some(Value::Object(stages)) = self.nodes.get("stages") else {
                return None;
            };
            let pos = stages.get_index_of(sid)?;
            return Some(((0, pos, sub), stages[pos].get(field)));
        }
        let i: usize = head
            .strip_prefix("inventory[")?
            .strip_suffix(']')?
            .parse()
            .ok()?;
        let Some(Value::List(items)) = self.nodes.get("inventory") else {
            return None;
        };
        Some(((1, i, sub), items.get(i)?.get(field)))
    }

    fn record(&mut self, kind: &str, inventory: bool, name: &str, what: &str) {
        self.calls.push(what.to_owned());
        let Some((key, value)) = self.locate(what) else {
            self.problems
                .push(format!("{kind}({name:?}, {what:?}) : champ inconnu"));
            return;
        };
        if !what.ends_with(&format!(".{kind}")) || inventory != (key.0 == 1) {
            self.problems.push(format!(
                "{kind}({name:?}, {what:?}, inventory={inventory}) : mauvais appel"
            ));
        }
        match value {
            Some(Value::Str(expected)) if !name.is_empty() && name == expected => {}
            other => self.problems.push(format!(
                "{kind}({name:?}, {what:?}) : attendu un nom non vide égal à {other:?}"
            )),
        }
        if self.last.is_some_and(|last| last >= key) {
            self.problems.push(format!(
                "{kind}({name:?}, {what:?}) : hors de l'ordre de Python"
            ));
        }
        self.last = Some(key);
    }

    fn get(&self, what: &str) -> Option<String> {
        // Champ inconnu : cas en échec avant `nodes.lua`, le nom n'a pas d'effet.
        self.py
            .get(what)
            .cloned()
            .unwrap_or_else(|| Some("x.lif".into()))
    }
}

impl Assets for Table<'_> {
    fn image(
        &mut self,
        name: &str,
        what: &str,
        inventory: bool,
    ) -> Result<Option<String>, ConvError> {
        self.record("image", inventory, name, what);
        Ok(self.get(what))
    }
    fn audio(&mut self, name: &str, what: &str) -> Result<Option<String>, ConvError> {
        self.record("audio", false, name, what);
        Ok(self.get(what))
    }
}

/// Rejoue un cas ; `Err` décrit l'écart.
fn replay(case: &Value) -> Result<(), String> {
    let warnings: Vec<&str> = list(field(case, "warnings")).iter().map(str_of).collect();
    let nodes = load_json(&raw(field(case, "nodes")).unwrap(), "nodes.json")
        .map_err(|e| format!("nodes.json refusé : {e}"))?;
    let notes_raw = raw(field(case, "notes"));
    let (notes, warning) = load_notes(notes_raw.as_deref());
    match case.get("divergence").map(str_of) {
        // JSON hors limites : Python lit `notes.json`, le Rust l'ignore avec un avertissement.
        Some("json_limit") => {
            if warning.is_none() || warnings.contains(&"notes_ignored") {
                return Err(format!(
                    "json_limit : Rust {warning:?}, Python {warnings:?} (écart attendu absent)"
                ));
            }
        }
        Some(other) => panic!("divergence inconnue : {other}"),
        None if warning.is_some() != warnings.contains(&"notes_ignored") => {
            return Err(format!(
                "notes_ignored : Rust {warning:?}, Python {warnings:?}"
            ));
        }
        None => {}
    }
    let py_assets: HashMap<String, Option<String>> = case
        .get("assets")
        .map(list)
        .unwrap_or_default()
        .iter()
        .map(|kv| {
            let kv = list(kv);
            (str_of(&kv[0]).to_owned(), opt_str(kv.get(1)))
        })
        .collect();
    let mut assets = Table::new(py_assets, &nodes);
    let header = case.get("header");
    let h = |k: &str| header.and_then(|h| opt_str(h.get(k)));
    let (title, subtitle) = (
        h("title").unwrap_or_default(),
        h("subtitle").unwrap_or_default(),
    );
    let (title_image, title_audio) = (h("title_image"), h("title_audio"));
    let selector = str_of(field(case, "selector")).to_owned();
    let allow_missing = matches!(field(case, "allow_missing"), Value::Bool(true));
    let missing = matches!(field(case, "missing"), Value::Bool(true));
    let mut replaced = Replaced::new();
    let mut graph_done = false;
    let result = (|| {
        let src = Sources::new(&nodes)?;
        let graph = convert_graph(&src, &notes, &mut replaced, &mut assets)?;
        graph_done = true;
        if missing && !allow_missing {
            return Err(ConvError {
                code: "missing_asset",
                exit_code: 5,
                message: String::new(),
            });
        }
        let start = Start::new(&src)?;
        let header = Header {
            title: &title,
            subtitle: &subtitle,
            title_image: title_image.as_deref(),
            title_audio: title_audio.as_deref(),
            selector: &selector,
        };
        nodes_lua(&header, &graph, &start)
    })();
    if !assets.problems.is_empty() {
        return Err(format!("appels à Assets : {:?}", assets.problems));
    }
    // Appels attendus : ceux que Python a faits (colonne `calls` du générateur, relevée dans
    // `check_asset_name`), pas une règle recalculée ici à partir de `nodes.json`.
    let expected_calls: Vec<&str> = list(field(case, "calls")).iter().map(str_of).collect();
    // Aussi quand le graphe échoue sur un nom d'asset refusé (#421) : Python relève l'appel dans
    // `check_asset_name` *avant* de rejeter le nom, son dernier appel est donc celui qui échoue ;
    // le Rust valide le nom avant d'appeler `Assets` et ne le fait pas. Les autres échecs ne
    // sont pas comparés (les deux moteurs peuvent s'arrêter à des endroits différents).
    let name_refused = case
        .get("error")
        .is_some_and(|e| str_of(field(e, "message")).contains("nom d'asset refuse"));
    let expected_calls = match expected_calls.split_last() {
        Some((_, before)) if !graph_done && name_refused => before,
        _ => &expected_calls[..],
    };
    if (graph_done || name_refused) && assets.calls != expected_calls {
        return Err(format!(
            "appels à Assets {:?}, attendus {expected_calls:?}",
            assets.calls
        ));
    }
    if !replaced.is_empty() && !warnings.contains(&"font_replaced") {
        return Err(format!("font_replaced absent chez Python : {replaced:?}"));
    }
    // Sens inverse : le `font_replaced` de Python vient du graphe ou de l'en-tête (titre,
    // sous-titre, hors de ce module) ; avec un en-tête ASCII, seul le graphe peut l'avoir levé.
    if replaced.is_empty()
        && warnings.contains(&"font_replaced")
        && header.is_some()
        && title.is_ascii()
        && subtitle.is_ascii()
    {
        return Err("font_replaced chez Python, rien de remplacé en Rust".into());
    }
    match (result, case.get("error")) {
        (Ok(lua), None) => {
            let expected = str_of(field(case, "nodes_lua"));
            let got = digest(&lua, 1500);
            if got != expected {
                return Err(format!(
                    "nodes.lua différent\n--- Rust\n{}\n--- Python\n{expected}",
                    String::from_utf8_lossy(&lua)
                ));
            }
            if str_of(field(case, "main_lua")) != digest(MAIN_LUA, 0) {
                return Err("main.lua différent".into());
            }
            Ok(())
        }
        (Err(e), Some(err)) => {
            let code = str_of(field(err, "code"));
            let exit = match field(err, "exit") {
                Value::Int(i) => *i,
                other => panic!("code de sortie : {other:?}"),
            };
            let message = str_of(field(err, "message"));
            if e.code != code || i64::from(e.exit_code) != exit {
                return Err(format!("erreur {e:?}, Python {code}/{exit} : {message}"));
            }
            // Message identique (repr de Python compris), sauf l'erreur simulée des assets
            // manquants.
            if code == "invalid_content" && e.message != message {
                return Err(format!("message « {} », Python « {message} »", e.message));
            }
            Ok(())
        }
        (Ok(_), Some(err)) => Err(format!("succès, Python en échec : {err:?}")),
        (Err(e), None) => Err(format!("échec {e:?}, Python a réussi")),
    }
}

#[test]
fn graphe_identique_a_python() {
    let t = table();
    assert_eq!(
        str_of(field(&t, "python")),
        "3.14.8",
        "table à régénérer avec le Python de ORACLE-VERSIONS.txt (3.14.8)"
    );
    let cases = list(field(&t, "cases"));
    assert!(cases.len() >= 250, "{} cas seulement", cases.len());
    let mut failures = Vec::new();
    let (mut ok, mut err) = (0, 0);
    for case in cases {
        match replay(case) {
            Ok(()) if case.get("error").is_some() => err += 1,
            Ok(()) => ok += 1,
            Err(why) => failures.push(format!("{} : {why}", str_of(field(case, "id")))),
        }
    }
    assert!(
        failures.is_empty(),
        "{} écart(s) :\n{}",
        failures.len(),
        failures.join("\n\n")
    );
    // Les cas d'écart assumé et les noms refusés sont bien dans la table.
    assert!(cases.iter().any(|c| c.get("divergence").is_some()));
    assert!(cases
        .iter()
        .any(|c| str_of(field(c, "id")).starts_with("g_bad_name_")));
    // Les deux familles sont couvertes (conversions réussies et erreurs du graphe).
    assert!(ok >= 150 && err >= 50, "{ok} réussites, {err} erreurs");
}
