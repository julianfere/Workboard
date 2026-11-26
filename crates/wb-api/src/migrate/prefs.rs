//! Preferencias de la ventana de la v1 (`config.json` → `prefs`, las escribe `setPref` de `ui/app.js`) en el
//! formato de la v2: `ui_prefs.json` (`grid`, `tasks`, `detail`, `hideHours`, `sessionWeightHidden`) y
//! `alerts_off.json`. Solo transformaciones puras; la escritura y el reporte están en `migrate.rs`.
//!
//! Los ids de la v1 (columnas, agrupaciones, tipos de alerta…) se traducen a los de la v2; estados, tipos y
//! ambientes se validan contra el preset. Lo que la v2 no conoce se deja afuera y se cuenta.

use std::collections::BTreeMap;

use serde_json::{Map, Value as Json, json};
use wb_core::Workflow;

/// Claves de `prefs` de la v1 que se migran (además de `theme`, que va a `settings.json`).
const MIGRATED: [&str; 26] = [
    "theme",
    "views",
    "activeView",
    "stateFilter",
    "typeFilter",
    "period",
    "pactFilter",
    "hideClosed",
    "groupBy",
    "gridCols",
    "gridSort",
    "density",
    "hideHours",
    "detailWidth",
    "detailWidthRestore",
    "tasksShowDone",
    "tasksState",
    "tasksSess",
    "tasksEnv",
    "tasksPact",
    "view",
    "hoursAll",
    "hoursSort",
    "mxRange",
    "mxTipo",
    "mxAll",
];
/// Claves por carpeta de trabajos (`<clave>@<raíz>`) que se migran.
const MIGRATED_PER_ROOT: [&str; 5] = ["alertsOff", "heavyOff", "pinnedSess", "keepSess", "hoursPeriod"];

/// ¿Queda en `prefs` algo que no se migra (última vista, grupos abiertos, Horas, Métricas…)?
pub fn has_unmigrated(prefs: &Map<String, Json>) -> bool {
    prefs.keys().any(|k| match k.split_once('@') {
        Some((base, _)) => !MIGRATED_PER_ROOT.contains(&base),
        None => !MIGRATED.contains(&k.as_str()),
    })
}

/// Valores de una clave por carpeta (`<base>@<raíz>`): `(raíz, valor)`.
pub fn per_root<'a>(prefs: &'a Map<String, Json>, base: &str) -> Vec<(&'a str, &'a Json)> {
    prefs
        .iter()
        .filter_map(|(k, v)| {
            let (b, root) = k.split_once('@')?;
            (b == base && !root.is_empty()).then_some((root, v))
        })
        .collect()
}

// ---------- grilla ----------

/// Columna de la v1 → la de la v2 (`rm` es el campo extra `release_manual` del flujo, si lo tiene).
fn column(wf: &Workflow, id: &str) -> Option<String> {
    let v2 = match id {
        "state" => "status",
        "wi" => "id",
        "tipo" => "type",
        "title" => "title",
        "folder" => "folder",
        "session" => "sessions",
        "notas" => "notes",
        "azure" => "tracker",
        "git" => "git",
        "kinds" => "kinds",
        "envs" => "envs",
        "rama" => "branch",
        "alerts" => "alerts",
        "last" => "last",
        "created" => "created",
        "prio" => "priority",
        "pactada" => "due",
        "horas" => "hours",
        "real" => "worked",
        "files" => "files",
        "rm" => {
            const KEY: &str = "release_manual";
            return wf.fields.extra.iter().any(|f| f.key == KEY).then(|| format!("x:{KEY}"));
        }
        _ => return None,
    };
    Some(v2.to_string())
}

/// Agrupación de la v1 → la de la v2.
fn group_key(g: &str) -> Option<&'static str> {
    Some(match g {
        "week-last" => "weekLast",
        "week-created" => "weekCreated",
        "periodo" => "period",
        "state" => "status",
        "tipo" => "type",
        "azure" => "tracker",
        "prio" => "priority",
        _ => return None,
    })
}

/// Vistas de fábrica de la v1 → las de la v2 (mismos filtros).
fn view_id(id: &str) -> &str {
    match id {
        "mia" => "mine",
        "activos" => "active",
        "qa" => "review",
        "todo" => "all",
        other => other,
    }
}

const DUE_FILTERS: [&str; 4] = ["", "upto", "cur", "late"];
const ACTIVITY_WEEKS: [&str; 5] = ["all", "2", "4", "8", "12"];

/// Filtros, agrupación, columnas, orden y densidad (un `Snap` de `grid.ts`) desde una vista de la v1 o las
/// prefs sueltas (los valores vigentes). Lo que falta toma el valor por defecto de la v1 (`VIEW_DEFAULTS`).
fn snap(wf: &Workflow, v: &Json, dropped: &mut usize) -> Json {
    let mut status = vec![];
    for s in v["stateFilter"].as_array().into_iter().flatten().filter_map(Json::as_str) {
        // `none` = sin estado (la v2 lo llama "").
        if s == "none" {
            status.push(String::new());
        } else if wf.states.iter().any(|x| x.id == s) {
            status.push(s.to_string());
        } else {
            *dropped += 1;
        }
    }
    let ty = match v["typeFilter"].as_str().unwrap_or("") {
        "" => String::new(),
        t if t == "none" || wf.types.iter().any(|x| x.id == t) => t.to_string(),
        _ => {
            *dropped += 1;
            String::new()
        }
    };
    let due = match v["pactFilter"].as_str().unwrap_or("") {
        d if DUE_FILTERS.contains(&d) => d.to_string(),
        _ => {
            *dropped += 1;
            String::new()
        }
    };
    let activity = match &v["period"] {
        Json::Null => "all".to_string(),
        Json::Number(n) => n.to_string(),
        other => other.as_str().unwrap_or("").to_string(),
    };
    let activity = if ACTIVITY_WEEKS.contains(&activity.as_str()) {
        activity
    } else {
        *dropped += 1;
        "all".to_string()
    };
    let groups: Vec<&Json> = match &v["groupBy"] {
        Json::Null => vec![],
        Json::Array(a) => a.iter().collect(),
        one => vec![one],
    };
    let mut group_by: Vec<&str> = vec![];
    if v["groupBy"].is_null() {
        group_by.push("weekLast");
    }
    for g in groups.into_iter().filter_map(Json::as_str).filter(|g| *g != "none") {
        match group_key(g) {
            Some(k) if !group_by.contains(&k) && group_by.len() < 3 => group_by.push(k),
            Some(_) => {}
            None => *dropped += 1,
        }
    }
    let columns = v["gridCols"].as_array().map(|cols| {
        let mut out: Vec<Json> = vec![];
        for c in cols {
            match c["id"].as_str().and_then(|id| column(wf, id)) {
                Some(id) if !out.iter().any(|x| x["id"] == id.as_str()) => {
                    let mut col = json!({"id": id});
                    if let Some(w) = c["w"].as_f64().filter(|w| *w > 0.0) {
                        col["w"] = json!(w.round() as i64);
                    }
                    out.push(col);
                }
                Some(_) => {}
                None => *dropped += 1,
            }
        }
        Json::Array(out)
    });
    let sort = match v["gridSort"]["id"].as_str() {
        None => Json::Null,
        Some(id) => match column(wf, id) {
            Some(id) => json!({"id": id, "dir": if v["gridSort"]["dir"] == "asc" { 1 } else { -1 }}),
            None => {
                *dropped += 1;
                Json::Null
            }
        },
    };
    json!({
        "status": status,
        "hideClosed": v["hideClosed"].as_bool().unwrap_or(true),
        "type": ty,
        "due": due,
        "activity": activity,
        "groupBy": group_by,
        "columns": columns.unwrap_or(Json::Null),
        "sort": sort,
        "density": if v["density"] == "compacta" { "compact" } else { "comfortable" },
    })
}

/// Claves de la grilla en las prefs sueltas de la v1.
const GRID_KEYS: [&str; 11] = [
    "views",
    "activeView",
    "stateFilter",
    "typeFilter",
    "period",
    "pactFilter",
    "hideClosed",
    "groupBy",
    "gridCols",
    "gridSort",
    "density",
];

/// `ui_prefs.json` → `grid` (como lo lee `ensurePrefs` de `grid.ts`): vistas guardadas, la activa y los
/// valores vigentes. Sin vistas en la v1, la v2 pone las suyas de fábrica. `None` si la v1 no tiene nada de
/// la grilla. Devuelve también cuántas vistas hay y cuántos valores se dejaron afuera.
pub fn grid(wf: &Workflow, prefs: &Map<String, Json>) -> Option<(Json, usize, usize)> {
    if !GRID_KEYS.iter().any(|k| prefs.contains_key(*k)) {
        return None;
    }
    let mut dropped = 0;
    let mut views: Vec<Json> = vec![];
    for v in prefs.get("views").and_then(Json::as_array).into_iter().flatten() {
        let Some(id) = v["id"].as_str().filter(|s| !s.is_empty()) else { continue };
        let id = view_id(id);
        if views.iter().any(|x| x["id"] == id) {
            continue;
        }
        let mut out = json!({"id": id, "name": v["name"].as_str().unwrap_or(id)});
        if let (Json::Object(o), Json::Object(s)) = (&mut out, snap(wf, v, &mut dropped)) {
            o.extend(s);
        }
        views.push(out);
    }
    let active = prefs
        .get("activeView")
        .and_then(Json::as_str)
        .map(view_id)
        .filter(|id| views.iter().any(|v| v["id"] == *id))
        .or_else(|| views.first().and_then(|v| v["id"].as_str()))
        .unwrap_or("")
        .to_string();
    let current = snap(wf, &Json::Object(prefs.clone()), &mut dropped);
    let count = views.len();
    let grid = json!({"views": views, "active": active, "current": current, "flipped": {}, "open": {}});
    Some((grid, count, dropped))
}

// ---------- Pendientes, detalle, modo discreto, sesiones largas ----------

/// `ui_prefs.json` → `tasks` (filtros de Pendientes, `tasks.ts`). `None` si la v1 no tiene ninguno.
pub fn tasks(wf: &Workflow, prefs: &Map<String, Json>) -> Option<Json> {
    let keys = ["tasksShowDone", "tasksState", "tasksSess", "tasksEnv", "tasksPact"];
    if !keys.iter().any(|k| prefs.contains_key(*k)) {
        return None;
    }
    let s = |k: &str| prefs.get(k).and_then(Json::as_str).unwrap_or("").to_string();
    let status = Some(s("tasksState")).filter(|v| wf.states.iter().any(|x| &x.id == v)).unwrap_or_default();
    let env = wf
        .environments
        .iter()
        .find(|e| e.id.eq_ignore_ascii_case(&s("tasksEnv")))
        .map(|e| e.id.clone())
        .unwrap_or_default();
    let due = Some(s("tasksPact")).filter(|d| DUE_FILTERS.contains(&d.as_str())).unwrap_or_default();
    Some(json!({
        "status": status,
        "session": s("tasksSess"),
        "env": env,
        "due": due,
        "showDone": prefs.get("tasksShowDone").and_then(Json::as_bool).unwrap_or(false),
    }))
}

/// Ancho mínimo del panel del detalle en la v2 (`PANEL_MIN` de `app.ts`).
const PANEL_MIN: i64 = 560;

/// `ui_prefs.json` → `detail` (`{width, max}`): la v1 guarda el ancho o `"max"` (y el ancho para volver).
pub fn detail(prefs: &Map<String, Json>) -> Option<Json> {
    let width = |v: Option<&Json>| v.and_then(Json::as_f64).map(|w| (w.round() as i64).max(PANEL_MIN));
    match prefs.get("detailWidth")? {
        Json::String(s) if s == "max" => {
            Some(json!({"width": width(prefs.get("detailWidthRestore")).unwrap_or(1100), "max": true}))
        }
        w => Some(json!({"width": width(Some(w))?, "max": false})),
    }
}

/// `ui_prefs.json` → `view` (la última vista, `startView` de `main.ts`): la v1 llamaba `hoy` a Hoy y `backlog`
/// a Trabajos (`VIEWS` de `app.js`).
pub fn view(prefs: &Map<String, Json>) -> Option<Json> {
    let v = match prefs.get("view")?.as_str()? {
        "hoy" => "today",
        "backlog" => "items",
        v @ ("tasks" | "hours" | "alerts" | "notes" | "metrics" | "tools" | "settings") => v,
        _ => return None,
    };
    Some(json!(v))
}

/// Órdenes de Horas (iguales en las dos versiones).
const HOURS_SORTS: [&str; 5] = ["dev", "plan", "inP", "total", "title"];

/// `ui_prefs.json` → `hours` = `{all, sort, period: {<raíz>: "AAAA-MM"}}` (`hours.ts`). La v1 guarda el período
/// por carpeta (`hoursPeriod@<raíz>`) como el instante de su inicio (ms, hora local, `periodStart`); su código es
/// el año y mes de ese día (`periodCode`). `root` pasa la raíz de la v1 a la de la v2.
pub fn hours(prefs: &Map<String, Json>, root: &dyn Fn(&str) -> String) -> Option<Json> {
    use chrono::{Datelike, TimeZone};
    let mut period = Map::new();
    for (r, ms) in per_root(prefs, "hoursPeriod") {
        let Some(ms) = ms.as_f64() else { continue };
        if let Some(d) = chrono::Local.timestamp_millis_opt(ms as i64).single() {
            period.insert(root(r), json!(format!("{:04}-{:02}", d.year(), d.month())));
        }
    }
    if period.is_empty() && !prefs.contains_key("hoursAll") && !prefs.contains_key("hoursSort") {
        return None;
    }
    let sort =
        prefs.get("hoursSort").and_then(Json::as_str).filter(|s| HOURS_SORTS.contains(s)).unwrap_or("dev");
    Some(json!({
        "all": prefs.get("hoursAll").and_then(Json::as_bool).unwrap_or(false),
        "sort": sort,
        "period": period,
    }))
}

/// `ui_prefs.json` → `metrics` = `{range, type, all}` (`metrics.ts`) desde `mxRange`, `mxTipo`, `mxAll`.
pub fn metrics(wf: &Workflow, prefs: &Map<String, Json>) -> Option<Json> {
    if !["mxRange", "mxTipo", "mxAll"].iter().any(|k| prefs.contains_key(*k)) {
        return None;
    }
    let range = prefs
        .get("mxRange")
        .and_then(Json::as_str)
        .filter(|r| ["30", "90", "all"].contains(r))
        .unwrap_or("90");
    let ty = prefs
        .get("mxTipo")
        .and_then(Json::as_str)
        .filter(|t| wf.types.iter().any(|x| x.id == *t))
        .unwrap_or("");
    Some(
        json!({"range": range, "type": ty, "all": prefs.get("mxAll").and_then(Json::as_bool).unwrap_or(false)}),
    )
}

/// `ui_prefs.json` → `sessionWeightHidden` (`{<sesión>: nivel}`): avisos de sesión larga ocultos. La v1 los
/// guarda por carpeta (`heavyOff@<raíz>`) con sus niveles (`larga`, `alta`); los ids de sesión son únicos.
pub fn weight_hidden(prefs: &Map<String, Json>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (_, map) in per_root(prefs, "heavyOff") {
        for (id, level) in map.as_object().into_iter().flatten() {
            let level = match level.as_str() {
                Some("larga") => "long",
                Some("alta") => "heavy",
                _ => continue,
            };
            out.insert(id.clone(), level.to_string());
        }
    }
    out
}

/// Ids de sesión de una lista por carpeta (`pinnedSess@<raíz>`, `keepSess@<raíz>`), de todas las carpetas.
pub fn session_ids(prefs: &Map<String, Json>, base: &str) -> std::collections::BTreeSet<String> {
    per_root(prefs, base)
        .into_iter()
        .flat_map(|(_, v)| v.as_array().into_iter().flatten().filter_map(Json::as_str).map(String::from))
        .collect()
}

// ---------- alertas desestimadas ----------

/// Tipo de alerta de la v1 (`ALERTS` de `app.js`) → el de la v2 (`KINDS` de `alerts.ts`).
fn alert_kind(code: &str) -> Option<&'static str> {
    Some(match code {
        "dup" => "dupes",
        "migration" => "finalEnv",
        "stale" => "stale",
        "nostate" => "noStatus",
        "noprefix" => "noPrefix",
        "noreadme" => "noReadme",
        "gitdesync" => "gitDesync",
        "gituncommitted" => "gitDirty",
        "planincomplete" => "plan",
        _ => return None,
    })
}

/// La huella de la v1 como la calcula la v2 (`fingerprint` de `alerts.ts`); `None` si no se puede
/// traducir (la alerta volvería a aparecer de todos modos).
fn alert_fp(wf: &Workflow, kind: &str, fp: &str) -> Option<String> {
    match kind {
        // Faltantes del plan: `pendientes` de la v1 es `tasks` (`PlanGap`).
        "plan" => fp
            .split(',')
            .filter(|g| !g.is_empty())
            .map(|g| match g {
                "plan" => Some("plan"),
                "pendientes" => Some("tasks"),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()
            .map(|v| v.join(",")),
        // Repos desparejos por ambiente: la v1 usa los ambientes en mayúsculas (`QA`) en el orden de los
        // repos; la v2, los ids del flujo en su orden (`gitDesyncs` de `git.ts`).
        "gitDesync" => {
            let list: Vec<Json> = serde_json::from_str(fp).ok()?;
            let mut out: Vec<(usize, Json)> = vec![];
            for d in list {
                let env = d["env"].as_str()?;
                let i = wf.environments.iter().position(|e| e.id.eq_ignore_ascii_case(env))?;
                out.push((i, json!({"env": wf.environments[i].id, "ok": d["ok"], "missing": d["missing"]})));
            }
            out.sort_by_key(|(i, _)| *i);
            serde_json::to_string(&out.into_iter().map(|(_, d)| d).collect::<Vec<_>>()).ok()
        }
        // El resto se calcula igual en las dos versiones (`gitDirty` tiene la misma forma).
        _ => Some(fp.to_string()),
    }
}

/// Las desestimadas de una carpeta (`alertsOff@<raíz>` = `{"<item>|<tipo>": huella}`) con las claves y
/// huellas de la v2. Devuelve también cuántas se dejaron afuera.
pub fn alerts_off(wf: &Workflow, map: &Json) -> (BTreeMap<String, String>, usize) {
    let mut out = BTreeMap::new();
    let mut dropped = 0;
    for (key, fp) in map.as_object().into_iter().flatten() {
        let converted = key.rsplit_once('|').and_then(|(name, code)| {
            let kind = alert_kind(code)?;
            Some((format!("{name}|{kind}"), alert_fp(wf, kind, fp.as_str()?)?))
        });
        match converted {
            Some((k, f)) => {
                out.insert(k, f);
            }
            None => dropped += 1,
        }
    }
    (out, dropped)
}
