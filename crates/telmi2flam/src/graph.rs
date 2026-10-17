//! Conversion du graphe `nodes.json` vers Lua (`script/nodes.lua`) et bootstrap `main.lua`,
//! comme la partie « Stages / Actions / Inventaire / startAction / nodes.lua » de `_convert`
//! dans `tools/telmi2flam/telmi2flam.py` (`_conv_trans`, `_conv_cond`, `_conv_item`,
//! `_max_chapters_per_run`).
//!
//! Le découpage suit l'ordre de Python, pour que la première erreur soit la même :
//! 1. [`Sources::new`] : `nodes.json stages` et `nodes.json inventory` (avant tout asset) ;
//! 2. [`convert_graph`] : stages (images et audios demandés à [`Assets`]), actions, inventaire ;
//! 3. (couverture, assets manquants, `font_replaced` : PR suivante, hors de ce module) ;
//! 4. [`Start::new`] : `startAction` ;
//! 5. [`nodes_lua`] : `totalChapters` et sérialisation.
//!
//! Invariants : aucune entrée ne fait paniquer le module ; le coût est linéaire en la taille de
//! `nodes.json` (le graphe des chapitres passe par des sommets « action » au lieu de relier
//! chaque stage à chaque entrée de son action, voir [`max_chapters_per_run`]) ; aucune
//! récursion pilotée par l'entrée (Tarjan itératif, tables Lua de profondeur fixe).

use indexmap::IndexMap;

use crate::archive::check_asset_name;
use crate::error::ConvError;
use crate::json::{expect, load_json, Kind, Object, Value};
use crate::lua::{lua_repr, py_float_repr, Lua};
use crate::text::{choice_label, text, Replaced};

/// Bootstrap du paquet (`engine/main.lua`), copié tel quel à la racine du paquet.
pub const MAIN_LUA: &[u8] = include_bytes!("../../../tools/telmi2flam/engine/main.lua");

/// En-tête de `script/nodes.lua`, identique à celui de Python (la parité compare les octets).
const NODES_LUA_HEADER: &str =
    "-- Genere par telmi2flam.py \u{2014} ne pas editer a la main\nreturn ";

/// Avertissement non bloquant (`_warn` de Python) : `code` stable de l'événement `warning`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    pub code: &'static str,
    pub message: String,
}

/// Résolution des assets référencés par le graphe (`conv_image` / `conv_audio` de Python,
/// après `check_asset_name`, que ce module applique). Appelée seulement pour un nom non vide,
/// dans l'ordre de Python. Rend le nom de sortie, ou `None` (image manquante : pas de clé
/// `image`, ou `nil` dans l'inventaire). Un nom de sortie vide est traité comme `None`.
pub trait Assets {
    fn image(
        &mut self,
        name: &str,
        what: &str,
        inventory: bool,
    ) -> Result<Option<String>, ConvError>;
    fn audio(&mut self, name: &str, what: &str) -> Result<Option<String>, ConvError>;
}

/// `notes.json` (optionnel) : objet des notes, ou objet vide et avertissement `notes_ignored`
/// s'il est illisible ou n'est pas un objet. `raw` vaut `None` si le fichier est absent.
pub fn load_notes(raw: Option<&[u8]>) -> (Object, Option<Warning>) {
    let ignored = |message: String| {
        Some(Warning {
            code: "notes_ignored",
            message,
        })
    };
    match raw
        .filter(|r| !r.is_empty())
        .map(|r| load_json(r, "notes.json"))
    {
        None => (Object::new(), None),
        Some(Ok(Value::Object(map))) => (map, None),
        Some(Ok(other)) => (
            Object::new(),
            ignored(format!(
                "notes.json ignore (objet attendu, {} trouve)",
                py_type_name(other.kind())
            )),
        ),
        Some(Err(e)) => (Object::new(), ignored(format!("notes.json ignore ({e})"))),
    }
}

/// `type(v).__name__` de Python pour une valeur JSON.
fn py_type_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Object => "dict",
        Kind::List => "list",
        Kind::Str => "str",
        Kind::Int => "int",
        Kind::Float => "float",
        Kind::Bool => "bool",
        Kind::Null => "NoneType",
    }
}

/// Vérité d'une valeur JSON au sens de Python (`if v:`).
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Int(i)) => *i != 0,
        Some(Value::UInt(u)) => *u != 0,
        Some(Value::Float(f)) => *f != 0.0,
        Some(Value::Str(s)) => !s.is_empty(),
        Some(Value::List(l)) => !l.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

/// `d.get(key, default)` : la valeur présente (même `null`), sinon `default`.
fn get_or<'a>(map: &'a Object, key: &str, default: &'a Value) -> &'a Value {
    map.get(key).unwrap_or(default)
}

fn expect_obj<'a>(v: Option<&'a Value>, what: &str) -> Result<&'a Object, ConvError> {
    match expect(v, &[Kind::Object], what, false)? {
        Some(Value::Object(m)) => Ok(m),
        _ => Err(ConvError::invalid_content(format!(
            "{what} : objet attendu"
        ))),
    }
}

fn expect_opt_obj<'a>(v: Option<&'a Value>, what: &str) -> Result<Option<&'a Object>, ConvError> {
    Ok(match expect(v, &[Kind::Object], what, true)? {
        Some(Value::Object(m)) => Some(m),
        _ => None,
    })
}

fn expect_opt_list<'a>(v: Option<&'a Value>, what: &str) -> Result<&'a [Value], ConvError> {
    Ok(match expect(v, &[Kind::List], what, true)? {
        Some(Value::List(l)) => l,
        _ => &[],
    })
}

fn expect_opt_str<'a>(v: Option<&'a Value>, what: &str) -> Result<Option<&'a str>, ConvError> {
    Ok(match expect(v, &[Kind::Str], what, true)? {
        Some(Value::Str(s)) => Some(s),
        _ => None,
    })
}

fn expect_str(v: Option<&Value>, what: &str) -> Result<String, ConvError> {
    match expect(v, &[Kind::Str], what, false)? {
        Some(Value::Str(s)) => Ok(s.clone()),
        _ => Err(ConvError::invalid_content(format!(
            "{what} : chaine attendu"
        ))),
    }
}

fn expect_int(v: &Value, what: &str) -> Result<i128, ConvError> {
    match expect(Some(v), &[Kind::Int], what, false)? {
        Some(Value::Int(i)) => Ok(i128::from(*i)),
        Some(Value::UInt(u)) => Ok(i128::from(*u)),
        _ => Err(ConvError::invalid_content(format!(
            "{what} : entier attendu"
        ))),
    }
}

fn expect_number(v: &Value, what: &str) -> Result<Number, ConvError> {
    match expect(Some(v), &[Kind::Int, Kind::Float], what, false)? {
        Some(Value::Int(i)) => Ok(Number::Int(i128::from(*i))),
        Some(Value::UInt(u)) => Ok(Number::Int(i128::from(*u))),
        Some(Value::Float(f)) => Ok(Number::Float(*f)),
        _ => Err(ConvError::invalid_content(format!(
            "{what} : entier/nombre attendu"
        ))),
    }
}

/// Nombre TELMI (`int` ou `float` de Python).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Number {
    Int(i128),
    Float(f64),
}

impl Number {
    fn lua(self) -> Lua {
        match self {
            Number::Int(i) => Lua::Int(i),
            Number::Float(f) => Lua::Float(f),
        }
    }
}

/// Cible d'une transition : `index`, ou `indexItem` (exclusifs, spec TELMI).
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Index(i128),
    IndexItem(i128),
}

/// Transition `{action, index|indexItem}` (`_conv_trans`).
#[derive(Debug, Clone, PartialEq)]
pub struct Transition {
    pub action: String,
    pub target: Target,
}

impl Transition {
    fn parse(tr: &Value, what: &str) -> Result<Self, ConvError> {
        let tr = expect_obj(Some(tr), what)?;
        let action = expect_str(tr.get("action"), &format!("{what}.action"))?;
        let target = match tr.get("indexItem") {
            Some(v) => Target::IndexItem(expect_int(v, &format!("{what}.indexItem"))?),
            None => Target::Index(expect_int(
                get_or(tr, "index", &Value::Int(0)),
                &format!("{what}.index"),
            )?),
        };
        Ok(Transition { action, target })
    }

    fn lua(&self) -> Lua {
        let target = match self.target {
            Target::Index(i) => ("index", i),
            Target::IndexItem(i) => ("indexItem", i),
        };
        table(vec![
            ("action", Lua::Str(self.action.clone())),
            (target.0, Lua::Int(target.1)),
        ])
    }
}

/// Commandes actives d'un stage (`control`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ctrl {
    pub ok: bool,
    pub home: bool,
    pub autoplay: bool,
}

/// Opération d'inventaire : valeur affectée.
#[derive(Debug, Clone, PartialEq)]
pub enum ItemValue {
    PlayingTime,
    AssignItem(i128),
    Number(Number),
}

/// Opération d'inventaire `{type, item, number|assignItem|playingTime}` (`_conv_item`).
#[derive(Debug, Clone, PartialEq)]
pub struct ItemOp {
    pub kind: i128,
    pub item: i128,
    pub value: ItemValue,
}

impl ItemOp {
    fn parse(it: &Value, what: &str) -> Result<Self, ConvError> {
        let it = expect_obj(Some(it), what)?;
        let zero = Value::Int(0);
        let kind = expect_int(get_or(it, "type", &zero), &format!("{what}.type"))?;
        let item = expect_int(get_or(it, "item", &zero), &format!("{what}.item"))?;
        let value = if truthy(it.get("playingTime")) {
            ItemValue::PlayingTime
        } else if let Some(v) = it.get("assignItem") {
            ItemValue::AssignItem(expect_int(v, &format!("{what}.assignItem"))?)
        } else {
            ItemValue::Number(expect_number(
                get_or(it, "number", &zero),
                &format!("{what}.number"),
            )?)
        };
        Ok(ItemOp { kind, item, value })
    }

    fn lua(&self) -> Lua {
        let value = match self.value {
            ItemValue::PlayingTime => ("playingTime", Lua::Bool(true)),
            ItemValue::AssignItem(i) => ("assignItem", Lua::Int(i)),
            ItemValue::Number(n) => ("number", n.lua()),
        };
        table(vec![
            ("type", Lua::Int(self.kind)),
            ("item", Lua::Int(self.item)),
            value,
        ])
    }
}

/// Stage converti, dans l'ordre des clés de Python.
#[derive(Debug, Clone, PartialEq)]
pub struct Stage {
    pub image: Option<String>,
    pub audio: Option<String>,
    pub ok: Option<Transition>,
    pub home: Option<Transition>,
    pub ctrl: Ctrl,
    pub items: Vec<ItemOp>,
    pub reset: bool,
    /// Label du choix (carrousel), vide si aucun.
    pub text: String,
}

impl Stage {
    fn lua(&self) -> Lua {
        let mut t = Vec::new();
        if let Some(i) = &self.image {
            t.push(("image", Lua::Str(i.clone())));
        }
        if let Some(a) = &self.audio {
            t.push(("audio", Lua::Str(a.clone())));
        }
        if let Some(ok) = &self.ok {
            t.push(("ok", ok.lua()));
        }
        if let Some(home) = &self.home {
            t.push(("home", home.lua()));
        }
        t.push((
            "ctrl",
            table(vec![
                ("ok", Lua::Bool(self.ctrl.ok)),
                ("home", Lua::Bool(self.ctrl.home)),
                ("autoplay", Lua::Bool(self.ctrl.autoplay)),
            ]),
        ));
        if !self.items.is_empty() {
            t.push((
                "items",
                Lua::List(self.items.iter().map(ItemOp::lua).collect()),
            ));
        }
        if self.reset {
            t.push(("reset", Lua::Bool(true)));
        }
        if !self.text.is_empty() {
            t.push(("text", Lua::Str(self.text.clone())));
        }
        table(t)
    }
}

/// Second membre d'une condition : un autre objet d'inventaire ou un nombre.
#[derive(Debug, Clone, PartialEq)]
pub enum CondValue {
    ItemB(i128),
    Num(Number),
}

/// Condition `{comparator, item, number|compareItem}` (`_conv_cond`).
#[derive(Debug, Clone, PartialEq)]
pub struct Cond {
    pub cmp: i128,
    pub item: i128,
    pub value: CondValue,
}

impl Cond {
    fn parse(c: &Value, what: &str) -> Result<Self, ConvError> {
        let c = expect_obj(Some(c), what)?;
        let zero = Value::Int(0);
        let cmp = expect_int(
            get_or(c, "comparator", &Value::Int(2)),
            &format!("{what}.comparator"),
        )?;
        let item = expect_int(get_or(c, "item", &zero), &format!("{what}.item"))?;
        let value = match c.get("compareItem") {
            Some(v) => CondValue::ItemB(expect_int(v, &format!("{what}.compareItem"))?),
            None => CondValue::Num(expect_number(
                get_or(c, "number", &zero),
                &format!("{what}.number"),
            )?),
        };
        Ok(Cond { cmp, item, value })
    }

    fn lua(&self) -> Lua {
        let value = match self.value {
            CondValue::ItemB(i) => ("itemB", Lua::Int(i)),
            CondValue::Num(n) => ("num", n.lua()),
        };
        table(vec![
            ("cmp", Lua::Int(self.cmp)),
            ("item", Lua::Int(self.item)),
            value,
        ])
    }
}

/// Entrée d'une action : stage cible et conditions (vides si aucune).
#[derive(Debug, Clone, PartialEq)]
pub struct ActionEntry {
    pub stage: String,
    pub cond: Vec<Cond>,
}

/// Objet d'inventaire converti.
#[derive(Debug, Clone, PartialEq)]
pub struct InventoryItem {
    pub name: String,
    pub init: Number,
    pub max: Number,
    pub display: i128,
    pub image: Option<String>,
}

/// Graphe converti : stages et actions dans l'ordre de `nodes.json`, inventaire (`None` si
/// absent ou vide, sérialisé `nil`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Graph {
    pub stages: IndexMap<String, Stage>,
    pub actions: IndexMap<String, Vec<ActionEntry>>,
    pub inventory: Option<Vec<InventoryItem>>,
}

/// Sections `stages` et `inventory` de `nodes.json`, validées avant tout asset comme en
/// Python (elles servent aussi aux totaux de progression).
#[derive(Debug, Clone, Copy)]
pub struct Sources<'a> {
    pub nodes: &'a Object,
    pub stages: Option<&'a Object>,
    pub inventory: &'a [Value],
}

impl<'a> Sources<'a> {
    /// `_expect(nodes, dict, "nodes.json")`, puis `stages` (objet) et `inventory` (liste).
    pub fn new(nodes: &'a Value) -> Result<Self, ConvError> {
        let nodes = expect_obj(Some(nodes), "nodes.json")?;
        let stages = expect_opt_obj(nodes.get("stages"), "nodes.json stages")?;
        let inventory = expect_opt_list(nodes.get("inventory"), "nodes.json inventory")?;
        Ok(Sources {
            nodes,
            stages,
            inventory,
        })
    }
}

/// `conv_image` / `conv_audio` : rien pour un nom absent ou vide, sinon nom vérifié puis
/// résolu par `assets`.
fn asset(
    name: Option<&str>,
    what: &str,
    resolve: impl FnOnce(&str, &str) -> Result<Option<String>, ConvError>,
) -> Result<Option<String>, ConvError> {
    let Some(name) = name.filter(|n| !n.is_empty()) else {
        return Ok(None);
    };
    check_asset_name(name, what)?;
    Ok(resolve(name, what)?.filter(|out| !out.is_empty()))
}

/// Stages, actions et inventaire (`_convert`, sections « Stages », « Actions »,
/// « Inventaire »). `replaced` reçoit les caractères hors police des labels et des noms
/// d'objets.
pub fn convert_graph(
    src: &Sources<'_>,
    notes: &Object,
    replaced: &mut Replaced,
    assets: &mut dyn Assets,
) -> Result<Graph, ConvError> {
    let mut graph = Graph::default();
    let no_note = Value::Null;
    for (sid, st) in src.stages.into_iter().flatten() {
        let what = format!("stage {sid}");
        let st = expect_obj(Some(st), &what)?;
        let w = format!("{what}.image");
        let image = asset(expect_opt_str(st.get("image"), &w)?, &w, |n, w| {
            assets.image(n, w, false)
        })?;
        let w = format!("{what}.audio");
        let audio = asset(expect_opt_str(st.get("audio"), &w)?, &w, |n, w| {
            assets.audio(n, w)
        })?;
        let ok = match st.get("ok") {
            v @ Some(tr) if truthy(v) => Some(Transition::parse(tr, &format!("{what}.ok"))?),
            _ => None,
        };
        let home = match st.get("home") {
            v @ Some(tr) if truthy(v) => Some(Transition::parse(tr, &format!("{what}.home"))?),
            _ => None,
        };
        let empty = Object::new();
        let ctrl = expect_opt_obj(st.get("control"), &format!("{what}.control"))?.unwrap_or(&empty);
        let ctrl = Ctrl {
            ok: ctrl.get("ok").is_none_or(|v| truthy(Some(v))),
            home: truthy(ctrl.get("home")),
            autoplay: truthy(ctrl.get("autoplay")),
        };
        let items = expect_opt_list(st.get("items"), &format!("{what}.items"))?
            .iter()
            .enumerate()
            .map(|(i, it)| ItemOp::parse(it, &format!("{what}.items[{i}]")))
            .collect::<Result<Vec<_>, _>>()?;
        let reset = truthy(st.get("inventoryReset"));
        let text = choice_label(
            notes.get(sid).unwrap_or(&no_note),
            Some(sid),
            Some(replaced),
        );
        graph.stages.insert(
            sid.clone(),
            Stage {
                image,
                audio,
                ok,
                home,
                ctrl,
                items,
                reset,
                text,
            },
        );
    }

    if let Some(actions) = expect_opt_obj(src.nodes.get("actions"), "nodes.json actions")? {
        for (aid, lst) in actions {
            let what = format!("action {aid}");
            let lst = match expect(Some(lst), &[Kind::List], &what, false)? {
                Some(Value::List(l)) => l,
                _ => {
                    return Err(ConvError::invalid_content(format!(
                        "{what} : liste attendu"
                    )))
                }
            };
            let mut out = Vec::with_capacity(lst.len());
            for (i, e) in lst.iter().enumerate() {
                let w = format!("{what}[{i}]");
                let e = expect_obj(Some(e), &w)?;
                let stage = expect_str(e.get("stage"), &format!("{w}.stage"))?;
                let cond = expect_opt_list(e.get("conditions"), &format!("{w}.conditions"))?
                    .iter()
                    .enumerate()
                    .map(|(j, c)| Cond::parse(c, &format!("{w}.conditions[{j}]")))
                    .collect::<Result<Vec<_>, _>>()?;
                out.push(ActionEntry { stage, cond });
            }
            graph.actions.insert(aid.clone(), out);
        }
    }

    if !src.inventory.is_empty() {
        let mut inventory = Vec::with_capacity(src.inventory.len());
        for (i, it) in src.inventory.iter().enumerate() {
            let what = format!("inventory[{i}]");
            let it = expect_obj(Some(it), &what)?;
            let w = format!("{what}.image");
            let image = asset(expect_opt_str(it.get("image"), &w)?, &w, |n, w| {
                assets.image(n, w, true)
            })?;
            let name = match it.get("name") {
                Some(Value::Str(s)) => text(s, Some(replaced)),
                Some(v @ (Value::Int(_) | Value::UInt(_) | Value::Float(_) | Value::Bool(_))) => {
                    text(&py_str_scalar(v), Some(replaced))
                }
                _ => String::new(),
            };
            let zero = Value::Int(0);
            inventory.push(InventoryItem {
                name,
                init: expect_number(
                    get_or(it, "initialNumber", &zero),
                    &format!("{what}.initialNumber"),
                )?,
                max: expect_number(get_or(it, "maxNumber", &zero), &format!("{what}.maxNumber"))?,
                display: expect_int(get_or(it, "display", &zero), &format!("{what}.display"))?,
                image,
            });
        }
        graph.inventory = Some(inventory);
    }
    Ok(graph)
}

/// `str(v)` de Python pour un scalaire numérique ou booléen (`bool` hérite de `int`).
fn py_str_scalar(v: &Value) -> String {
    match v {
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        Value::Int(i) => i.to_string(),
        Value::UInt(u) => u.to_string(),
        Value::Float(f) => py_float_repr(*f),
        _ => String::new(),
    }
}

/// Action de départ (`startAction`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Start {
    pub action: String,
    pub index: i128,
}

impl Start {
    /// `startAction` de `nodes.json` (obligatoire).
    pub fn new(src: &Sources<'_>) -> Result<Self, ConvError> {
        let sa = expect_obj(src.nodes.get("startAction"), "nodes.json startAction")?;
        Ok(Start {
            action: expect_str(sa.get("action"), "startAction.action")?,
            index: expect_int(get_or(sa, "index", &Value::Int(0)), "startAction.index")?,
        })
    }
}

/// Chapitres : stages avec audio qui peuvent être joués comme scène (`enterStage` de
/// `story.lua`) : cible d'une action à une seule entrée, d'une action à conditions, ou stage
/// autoplay. Indicateur par stage, dans l'ordre de `graph.stages`.
fn chapters(graph: &Graph) -> Vec<bool> {
    let mut playable = vec![false; graph.stages.len()];
    for lst in graph.actions.values() {
        let direct = lst.len() <= 1 || lst.iter().any(|e| !e.cond.is_empty());
        for e in lst {
            if let Some((i, _, st)) = graph.stages.get_full(&e.stage) {
                if direct || st.ctrl.autoplay {
                    playable[i] = true;
                }
            }
        }
    }
    graph
        .stages
        .values()
        .zip(playable)
        .map(|(st, p)| p && st.audio.is_some())
        .collect()
}

/// `_max_chapters_per_run` : nombre maximal de chapitres visitables en une partie, depuis
/// les entrées de l'action de départ (borne haute : conditions ignorées).
///
/// Python relie chaque stage à chaque entrée de l'action de son `ok` (et de son `home` si
/// `ctrl.home`) : `S` stages vers une même action de `E` entrées font `S × E` arcs. Ici,
/// chaque action est un sommet de poids nul (stage → action → stages) : `O(S + A + E)` arcs.
/// Les composantes fortement connexes ont les mêmes stages et les chemins le même poids,
/// donc le résultat est celui de Python (vérifié par les vecteurs et un test aléatoire).
pub fn max_chapters_per_run(graph: &Graph, start: &Start) -> usize {
    let ns = graph.stages.len();
    let n = ns + graph.actions.len();
    let weight: Vec<bool> = chapters(graph);
    // Graphe en CSR : arcs de chaque sommet.
    let mut adj_start = Vec::with_capacity(n + 1);
    let mut adj = Vec::new();
    for st in graph.stages.values() {
        adj_start.push(adj.len());
        for tr in [Some(&st.ok), st.ctrl.home.then_some(&st.home)]
            .into_iter()
            .flatten()
            .flatten()
        {
            if let Some(a) = graph.actions.get_index_of(&tr.action) {
                adj.push(ns + a);
            }
        }
    }
    for lst in graph.actions.values() {
        adj_start.push(adj.len());
        adj.extend(
            lst.iter()
                .filter_map(|e| graph.stages.get_index_of(&e.stage)),
        );
    }
    adj_start.push(adj.len());
    let succ = |v: usize| &adj[adj_start[v]..adj_start[v + 1]];

    let roots: Vec<usize> = graph
        .actions
        .get(&start.action)
        .into_iter()
        .flatten()
        .filter_map(|e| graph.stages.get_index_of(&e.stage))
        .collect();

    // Tarjan itératif : pas de récursion (graphes de plusieurs milliers de stages).
    const NONE: usize = usize::MAX;
    let mut index = vec![NONE; n];
    let mut low = vec![0usize; n];
    let mut comp = vec![NONE; n];
    let mut on_stack = vec![false; n];
    let mut stack = Vec::new();
    let mut best: Vec<usize> = Vec::new();
    let mut counter = 0usize;
    for &root in &roots {
        if index[root] != NONE {
            continue;
        }
        index[root] = counter;
        low[root] = counter;
        counter += 1;
        stack.push(root);
        on_stack[root] = true;
        // (sommet, position du prochain successeur)
        let mut work = vec![(root, 0usize)];
        while let Some(&mut (v, ref mut pos)) = work.last_mut() {
            if let Some(&w) = succ(v).get(*pos) {
                *pos += 1;
                if index[w] == NONE {
                    index[w] = counter;
                    low[w] = counter;
                    counter += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    work.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
                continue;
            }
            work.pop();
            if let Some(&(u, _)) = work.last() {
                low[u] = low[u].min(low[v]);
            }
            if low[v] == index[v] {
                // Composante émise en ordre topologique inverse : les composantes
                // atteignables depuis elle ont déjà leur meilleur chemin.
                let c = best.len();
                let mut members = Vec::new();
                while let Some(w) = stack.pop() {
                    on_stack[w] = false;
                    comp[w] = c;
                    members.push(w);
                    if w == v {
                        break;
                    }
                }
                let own = members.iter().filter(|&&s| s < ns && weight[s]).count();
                let next = members
                    .iter()
                    .flat_map(|&s| succ(s).iter())
                    .filter(|&&w| comp[w] != c)
                    .map(|&w| best[comp[w]])
                    .max()
                    .unwrap_or(0);
                best.push(own + next);
            }
        }
    }
    roots.iter().map(|&r| best[comp[r]]).max().unwrap_or(0)
}

fn table(items: Vec<(&str, Lua)>) -> Lua {
    Lua::Table(
        items
            .into_iter()
            .map(|(k, v)| (Lua::Str(k.to_owned()), v))
            .collect(),
    )
}

/// Données de `nodes.lua` qui ne viennent pas de `nodes.json`.
#[derive(Debug, Clone, Copy)]
pub struct Header<'a> {
    /// Titre affiché (`_text` du titre de `metadata.json`, ou « Histoire »).
    pub title: &'a str,
    /// Sous-titre affiché (catégorie), vide si aucun.
    pub subtitle: &'a str,
    /// Image de titre (`title.lif`) si la couverture a été convertie.
    pub title_image: Option<&'a str>,
    /// Audio de titre (`title.mp3`) s'il existe.
    pub title_audio: Option<&'a str>,
    /// Sélecteur de choix (`carousel` ou `image`).
    pub selector: &'a str,
}

/// Contenu de `script/nodes.lua`, octet pour octet comme Python.
pub fn nodes_lua(header: &Header<'_>, graph: &Graph, start: &Start) -> Result<Vec<u8>, ConvError> {
    let total = max_chapters_per_run(graph, start).max(1);
    let title = if header.title_image.is_some() || header.title_audio.is_some() {
        let mut t = Vec::new();
        if let Some(i) = header.title_image {
            t.push(("image", Lua::Str(i.to_owned())));
        }
        if let Some(a) = header.title_audio {
            t.push(("audio", Lua::Str(a.to_owned())));
        }
        table(t)
    } else {
        Lua::Nil
    };
    let inventory = match &graph.inventory {
        None => Lua::Nil,
        Some(items) => Lua::List(
            items
                .iter()
                .map(|it| {
                    table(vec![
                        ("name", Lua::Str(it.name.clone())),
                        ("init", it.init.lua()),
                        ("max", it.max.lua()),
                        ("display", Lua::Int(it.display)),
                        ("image", it.image.clone().map_or(Lua::Nil, Lua::Str)),
                    ])
                })
                .collect(),
        ),
    };
    let stages = Lua::Table(
        graph
            .stages
            .iter()
            .map(|(sid, st)| (Lua::Str(sid.clone()), st.lua()))
            .collect(),
    );
    let actions = Lua::Table(
        graph
            .actions
            .iter()
            .map(|(aid, lst)| {
                let entries = lst
                    .iter()
                    .map(|e| {
                        let mut t = vec![("stage", Lua::Str(e.stage.clone()))];
                        if !e.cond.is_empty() {
                            t.push(("cond", Lua::List(e.cond.iter().map(Cond::lua).collect())));
                        }
                        table(t)
                    })
                    .collect();
                (Lua::Str(aid.clone()), Lua::List(entries))
            })
            .collect(),
    );
    let data = table(vec![
        (
            "meta",
            table(vec![
                ("title", Lua::Str(header.title.to_owned())),
                ("subtitle", Lua::Str(header.subtitle.to_owned())),
            ]),
        ),
        // `usize` vers `i128` : au plus le nombre de stages, donc sans perte.
        (
            "totalChapters",
            Lua::Int(i128::try_from(total).unwrap_or(i128::MAX)),
        ),
        (
            "start",
            table(vec![
                ("action", Lua::Str(start.action.clone())),
                ("index", Lua::Int(start.index)),
            ]),
        ),
        ("title", title),
        ("inventory", inventory),
        ("stages", stages),
        ("actions", actions),
        ("selector", Lua::Str(header.selector.to_owned())),
    ]);
    let body =
        lua_repr(&data, 0).map_err(|e| ConvError::invalid_content(format!("nodes.lua : {e}")))?;
    let mut out = String::with_capacity(NODES_LUA_HEADER.len() + body.len() + 1);
    out.push_str(NODES_LUA_HEADER);
    out.push_str(&body);
    out.push('\n');
    Ok(out.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage(ok: Option<&str>, home: Option<&str>, ctrl_home: bool, audio: bool) -> Stage {
        let tr = |a: &str| Transition {
            action: a.to_owned(),
            target: Target::Index(0),
        };
        Stage {
            image: None,
            audio: audio.then(|| "a.mp3".to_owned()),
            ok: ok.map(tr),
            home: home.map(tr),
            ctrl: Ctrl {
                ok: true,
                home: ctrl_home,
                autoplay: true,
            },
            items: Vec::new(),
            reset: false,
            text: String::new(),
        }
    }

    fn entry(s: &str) -> ActionEntry {
        ActionEntry {
            stage: s.to_owned(),
            cond: Vec::new(),
        }
    }

    fn start(a: &str) -> Start {
        Start {
            action: a.to_owned(),
            index: 0,
        }
    }

    /// Générateur pseudo-aléatoire déterministe (LCG), sans dépendance.
    struct Lcg(u64);

    impl Lcg {
        fn below(&mut self, n: usize) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((self.0 >> 33) % n as u64) as usize
        }
    }

    /// Successeurs d'un stage comme `succ` de Python (arcs stage -> stage, doublons compris).
    fn py_succ(g: &Graph, i: usize) -> Vec<usize> {
        let s = &g.stages[i];
        [Some(&s.ok), s.ctrl.home.then_some(&s.home)]
            .into_iter()
            .flatten()
            .flatten()
            .flat_map(|tr| g.actions.get(&tr.action).into_iter().flatten())
            .filter_map(|e| g.stages.get_index_of(&e.stage))
            .collect()
    }

    /// Référence directe de `_max_chapters_per_run` : composantes par fermeture transitive,
    /// plus long chemin du graphe condensé par mémoïsation.
    fn reference(g: &Graph, st: &Start) -> usize {
        let n = g.stages.len();
        let weight = chapters(g);
        let mut reach = vec![vec![false; n]; n];
        for (i, row) in reach.iter_mut().enumerate() {
            let mut todo = vec![i];
            while let Some(v) = todo.pop() {
                for w in py_succ(g, v) {
                    if !row[w] {
                        row[w] = true;
                        todo.push(w);
                    }
                }
            }
        }
        let same = |a: usize, b: usize| a == b || (reach[a][b] && reach[b][a]);
        let mut memo: Vec<Option<usize>> = vec![None; n];
        // Ordre de calcul : un sommet après tous ceux qu'il atteint hors de sa composante.
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by_key(|&v| (0..n).filter(|&w| reach[v][w] && !same(v, w)).count());
        for &v in &order {
            let members: Vec<usize> = (0..n).filter(|&m| same(v, m)).collect();
            let own = members.iter().filter(|&&m| weight[m]).count();
            let next = members
                .iter()
                .flat_map(|&m| py_succ(g, m))
                .filter(|&w| !same(v, w))
                .map(|w| memo[w].expect("successeur calculé avant"))
                .max()
                .unwrap_or(0);
            memo[v] = Some(own + next);
        }
        g.actions
            .get(&st.action)
            .into_iter()
            .flatten()
            .filter_map(|e| g.stages.get_index_of(&e.stage))
            .map(|r| memo[r].unwrap_or(0))
            .max()
            .unwrap_or(0)
    }

    #[test]
    fn chapitres_comme_la_reference_sur_graphes_aleatoires() {
        let mut rng = Lcg(20_261_017);
        for _ in 0..3000 {
            let ns = 1 + rng.below(9);
            let na = 1 + rng.below(6);
            let mut g = Graph::default();
            let act = |i: usize| format!("a{i}");
            for i in 0..ns {
                let ok = (rng.below(5) > 0).then(|| act(rng.below(na + 1)));
                let home = (rng.below(3) == 0).then(|| act(rng.below(na + 1)));
                let (ctrl_home, audio) = (rng.below(2) == 0, rng.below(3) > 0);
                let mut st = stage(ok.as_deref(), home.as_deref(), ctrl_home, audio);
                st.ctrl.autoplay = rng.below(2) == 0;
                g.stages.insert(format!("s{i}"), st);
            }
            for a in 0..na {
                let len = rng.below(4);
                let mut lst: Vec<ActionEntry> = (0..len)
                    .map(|_| entry(&format!("s{}", rng.below(ns + 1))))
                    .collect();
                if rng.below(4) == 0 {
                    if let Some(e) = lst.first_mut() {
                        e.cond.push(Cond {
                            cmp: 0,
                            item: 0,
                            value: CondValue::Num(Number::Int(1)),
                        });
                    }
                }
                g.actions.insert(act(a), lst);
            }
            let st = start(&act(rng.below(na + 1)));
            assert_eq!(max_chapters_per_run(&g, &st), reference(&g, &st), "{g:?}");
        }
    }

    #[test]
    fn longue_chaine_sans_recursion() {
        // 200 000 stages en chaîne : la pile de Tarjan est un vecteur, pas la pile d'appels.
        let n = 200_000;
        let mut g = Graph::default();
        for i in 0..n {
            let next = format!("a{}", i + 1);
            g.stages
                .insert(format!("s{i}"), stage(Some(&next), None, false, true));
            g.actions
                .insert(format!("a{i}"), vec![entry(&format!("s{i}"))]);
        }
        assert_eq!(max_chapters_per_run(&g, &start("a0")), n);
        // Fermée en cycle : une seule composante.
        g.stages[n - 1].ok = Some(Transition {
            action: "a0".into(),
            target: Target::Index(0),
        });
        assert_eq!(max_chapters_per_run(&g, &start("a0")), n);
    }

    #[test]
    fn etoile_lineaire() {
        // S stages vers une même action de S entrées : S² arcs chez Python, 2S ici.
        let n = 50_000;
        let mut g = Graph::default();
        for i in 0..n {
            g.stages
                .insert(format!("s{i}"), stage(Some("hub"), Some("hub"), true, true));
        }
        let hub = (0..n).map(|i| entry(&format!("s{i}"))).collect();
        g.actions.insert("hub".into(), hub);
        assert_eq!(max_chapters_per_run(&g, &start("hub")), n);
        let header = Header {
            title: "T",
            subtitle: "",
            title_image: None,
            title_audio: None,
            selector: "carousel",
        };
        let lua = nodes_lua(&header, &g, &start("hub")).unwrap();
        assert!(lua.starts_with(NODES_LUA_HEADER.as_bytes()));
    }

    #[test]
    fn references_absentes_et_indices_hors_bornes() {
        let mut g = Graph::default();
        g.stages.insert(
            "s0".into(),
            stage(Some("absente"), Some("absente"), true, true),
        );
        let entries = vec![entry("fantome"), entry("s0"), entry("s0")];
        g.actions.insert("a0".into(), entries);
        g.stages[0].ok.as_mut().unwrap().target = Target::IndexItem(i128::from(u64::MAX));
        // s0 est cible d'une action à plusieurs entrées, mais autoplay : chapitre.
        assert_eq!(max_chapters_per_run(&g, &start("a0")), 1);
        assert_eq!(max_chapters_per_run(&g, &start("inconnue")), 0);
        assert_eq!(max_chapters_per_run(&Graph::default(), &start("a0")), 0);
    }

    struct FakeAssets;

    impl Assets for FakeAssets {
        fn image(&mut self, n: &str, _: &str, _: bool) -> Result<Option<String>, ConvError> {
            Ok(Some(format!("{n}.lif")))
        }
        fn audio(&mut self, _: &str, _: &str) -> Result<Option<String>, ConvError> {
            Ok(Some(String::new()))
        }
    }

    fn run(nodes: &str) -> Result<Vec<u8>, ConvError> {
        let nodes = load_json(nodes.as_bytes(), "nodes.json")?;
        let src = Sources::new(&nodes)?;
        let mut replaced = Replaced::new();
        let graph = convert_graph(&src, &Object::new(), &mut replaced, &mut FakeAssets)?;
        let header = Header {
            title: "T",
            subtitle: "",
            title_image: None,
            title_audio: None,
            selector: "carousel",
        };
        nodes_lua(&header, &graph, &Start::new(&src)?)
    }

    #[test]
    fn entrees_hostiles_sans_panique() {
        let deep = format!("{}1{}", "[".repeat(400), "]".repeat(400));
        let extremes = r#"{"stages": {"s": {"items": [{"type": -9223372036854775808,
            "item": 9223372036854775807}]}}, "startAction": {"action": "a"}}"#;
        for nodes in [
            "[]".to_owned(),
            "{}".to_owned(),
            extremes.to_owned(),
            format!(r#"{{"stages": {{"s": {{"ok": {deep}}}}}}}"#),
            format!(r#"{{"stages": {{"s": {{"items": {deep}}}}}}}"#),
            format!(r#"{{"actions": {{"a": [{{"stage": "s", "conditions": {deep}}}]}}}}"#),
            format!(r#"{{"inventory": [{{"name": {deep}}}], "startAction": {{"action": "a"}}}}"#),
            r#"{"stages": {"s": {"image": "../x.png"}}}"#.to_owned(),
        ] {
            let _ = run(&nodes);
        }
        assert!(run(extremes).is_ok());
        // Nom de sortie vide rendu par `Assets` : pas de clé `audio`, comme `if aud:`.
        let lua = run(r#"{"stages": {"s": {"audio": "x.mp3", "image": "y.png"}},
                "actions": {"a": [{"stage": "s"}]}, "startAction": {"action": "a"}}"#)
        .unwrap();
        let lua = String::from_utf8(lua).unwrap();
        assert!(lua.contains("image = \"y.png.lif\"") && !lua.contains("audio ="));
        assert!(lua.contains("totalChapters = 1,"));
    }

    #[test]
    fn notes_illisibles_ignorees() {
        assert_eq!(load_notes(None), (Object::new(), None));
        assert_eq!(load_notes(Some(b"")), (Object::new(), None));
        for (raw, msg) in [
            (&b"[]"[..], "notes.json ignore (objet attendu, list trouve)"),
            (
                b"null",
                "notes.json ignore (objet attendu, NoneType trouve)",
            ),
            (b"true", "notes.json ignore (objet attendu, bool trouve)"),
        ] {
            let (notes, w) = load_notes(Some(raw));
            assert!(notes.is_empty());
            let expected = Warning {
                code: "notes_ignored",
                message: msg.to_owned(),
            };
            assert_eq!(w, Some(expected));
        }
        assert_eq!(load_notes(Some(b"{oops")).1.unwrap().code, "notes_ignored");
    }
}
