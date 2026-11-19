//! Azure DevOps (REST 7.0), como `ado.py` de la v1.
//!
//! - Lectura: `GET` a `_apis/wit/workitemtypes`, `_apis/wit/fields` y `_apis/wit/workitems` (en lotes de
//!   200), más el `POST` de solo lectura a `_apis/wit/wiql` (la API no tiene `GET` para consultas ad hoc;
//!   alcanza con el scope Work Items: Read).
//! - Escritura (solo [`Tracker::set_state`], con la credencial de escritura): `PATCH` de `System.State`.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use base64::Engine;
use regex::Regex;
use serde_json::{Value as Json, json};
use wb_core::StateCategory;

use crate::html::html_to_text;
use crate::{
    Detail, FieldInfo, FieldRequest, HttpRequest, Result, StateChange, Tracker, TrackerError, TrackerState,
    Transport, WorkItem,
};

pub const KIND: &str = "azure-devops";
pub const SOURCE: &str = "Azure DevOps";
const API_VERSION: &str = "7.0";
/// Máximo de ids por pedido que acepta la API.
pub const BATCH: usize = 200;
const BASE_FIELDS: &[&str] = &[
    "System.Id",
    "System.State",
    "System.Title",
    "System.WorkItemType",
    "System.AssignedTo",
    "System.ChangedDate",
];
pub const PRIORITY_FIELD: &str = "Microsoft.VSTS.Common.Priority";
/// Estimación original (procesos Agile/CMMI): candidato a horas pactadas.
pub const ESTIMATE_FIELD: &str = "Microsoft.VSTS.Scheduling.OriginalEstimate";
pub const COMPLETED_FIELD: &str = "Microsoft.VSTS.Scheduling.CompletedWork";
const DESCRIPTION_FIELD: &str = "System.Description";
const CRITERIA_FIELD: &str = "Microsoft.VSTS.Common.AcceptanceCriteria";
const REPRO_FIELD: &str = "Microsoft.VSTS.TCM.ReproSteps";
/// Para autoelegir la fecha pactada entre los campos de fecha (nombre o referencia, en este orden; gana
/// la primera palabra que coincide con un solo campo). Las primeras son las de la v1.
pub const DUE_GUESS: &[&str] =
    &["pactad", "estimada finalizacion", "fechaestimadafinalizacion", "target date", "due date"];
/// Candidatos a horas pactadas entre los campos numéricos (nombre o referencia en minúsculas, `_` y `.`
/// como espacios). Gana el que más work items tienen cargado; la estimación original entra siempre.
pub const HOURS_GUESS: &[&str] =
    &["pactad", "horas", "hs ", "duracion", "estimate", "estimacion", "effort", "hours"];
const CATEGORY_ORDER: &[&str] = &["Proposed", "InProgress", "Resolved", "Completed", "Removed"];

/// Categoría de estado de Azure → categoría del flujo.
pub fn category_of(category: &str) -> Option<StateCategory> {
    Some(match category {
        "Proposed" => StateCategory::Backlog,
        "InProgress" => StateCategory::Active,
        "Resolved" => StateCategory::Review,
        "Completed" => StateCategory::Done,
        "Removed" => StateCategory::Cancelled,
        _ => return None,
    })
}

/// Los campos del proyecto que importan para planificar.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Catalog {
    pub has_priority: bool,
    pub has_effort: bool,
    /// Fechas: primero los propios del proyecto, después los de sistema; por nombre.
    pub date_fields: Vec<FieldInfo>,
    /// Numéricos (sin los de sistema): primero los propios del proyecto; por nombre.
    pub num_fields: Vec<FieldInfo>,
    /// Fecha pactada autoelegida (ver [`DUE_GUESS`]).
    pub due_guess: Option<String>,
}

pub fn catalog(fields: &[FieldInfo]) -> Catalog {
    let mut nums: Vec<FieldInfo> = fields
        .iter()
        .filter(|f| matches!(f.kind.as_str(), "double" | "integer") && !f.reference.starts_with("System."))
        .cloned()
        .collect();
    nums.sort_by_key(|f| (f.reference.starts_with("Microsoft."), f.name.to_lowercase()));
    let mut dates: Vec<FieldInfo> = fields.iter().filter(|f| f.kind == "dateTime").cloned().collect();
    dates.sort_by_key(|f| {
        (f.reference.starts_with("System.") || f.reference.starts_with("Microsoft."), f.name.to_lowercase())
    });
    let due_guess = DUE_GUESS.iter().find_map(|word| {
        let hits: Vec<&FieldInfo> = dates
            .iter()
            .filter(|f| format!("{} {}", f.name, f.reference).to_lowercase().contains(word))
            .collect();
        (hits.len() == 1).then(|| hits[0].reference.clone())
    });
    Catalog {
        has_priority: fields.iter().any(|f| f.reference == PRIORITY_FIELD),
        has_effort: fields.iter().any(|f| f.reference == COMPLETED_FIELD),
        date_fields: dates,
        num_fields: nums,
        due_guess,
    }
}

/// Campos numéricos que suenan a horas pactadas (como mucho 40), más la estimación original.
pub fn hours_candidates(num_fields: &[FieldInfo]) -> Vec<String> {
    num_fields
        .iter()
        .filter(|f| {
            let norm = format!("{} {} ", f.name, f.reference).to_lowercase().replace(['_', '.'], " ");
            f.reference == ESTIMATE_FIELD || HOURS_GUESS.iter().any(|w| norm.contains(w))
        })
        .take(40)
        .map(|f| f.reference.clone())
        .collect()
}

/// El candidato con más work items cargados (> 0); si ninguno tiene, la estimación original (o "").
pub fn pick_hours(cands: &[String], items: &BTreeMap<String, WorkItem>) -> String {
    let count = |c: &String| {
        items.values().filter(|w| w.fields.get(c).and_then(Json::as_f64).is_some_and(|v| v > 0.0)).count()
    };
    let mut best: Option<(&String, usize)> = None;
    for c in cands {
        let n = count(c);
        if best.is_none_or(|(_, b)| n > b) {
            best = Some((c, n));
        }
    }
    match best {
        Some((c, n)) if n > 0 => c.clone(),
        _ if cands.iter().any(|c| c == ESTIMATE_FIELD) => ESTIMATE_FIELD.to_string(),
        _ => String::new(),
    }
}

/// `https://<org>.visualstudio.com` o `https://dev.azure.com/<org>` (sin `/` final).
pub fn normalize_org(org: &str) -> Result<String> {
    let mut org = org.trim().trim_end_matches('/').to_string();
    if org.is_empty() {
        return Err(TrackerError::new("tracker.noOrg", vec![]));
    }
    if !org.starts_with("http") {
        org = format!("https://{org}");
    }
    let bad = || TrackerError::new("tracker.badOrg", vec![]);
    let rest = org.strip_prefix("https://").ok_or_else(bad)?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default().to_lowercase();
    if host.contains(['@', ':']) || !(host == "dev.azure.com" || host.ends_with(".visualstudio.com")) {
        return Err(bad());
    }
    if rest.contains(['?', '#'])
        || (host == "dev.azure.com" && rest.split('/').nth(1).is_none_or(str::is_empty))
    {
        return Err(bad());
    }
    Ok(org)
}

pub fn normalize_project(project: &str) -> Result<String> {
    let project = project.trim();
    if project.is_empty() {
        return Err(TrackerError::new("tracker.noProject", vec![]));
    }
    Ok(project.to_string())
}

/// Página de un work item.
pub fn item_url(org: &str, id: &str) -> String {
    format!("{org}/_workitems/edit/{id}")
}

/// Codifica un segmento de URL (todo menos los caracteres no reservados, más los de `keep`).
fn pct(s: &str, keep: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) || keep.as_bytes().contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn is_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 10 && id.bytes().all(|b| b.is_ascii_digit())
}

fn field_ref_ok(r: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[A-Za-z][A-Za-z0-9_.]*$").expect("regex")).is_match(r)
}

fn text(v: &Json) -> String {
    v.as_str().unwrap_or_default().to_string()
}

fn number(v: &Json) -> Option<f64> {
    v.as_f64()
}

pub struct AzureDevOps {
    org: String,
    project: String,
    auth: String,
    secret: String,
    http: Arc<dyn Transport>,
}

impl AzureDevOps {
    pub fn new(org: &str, project: &str, pat: &str, http: Arc<dyn Transport>) -> Result<Self> {
        let pat = pat.trim();
        if pat.is_empty() {
            return Err(TrackerError::new("tracker.noSecret", vec![]));
        }
        let token = base64::engine::general_purpose::STANDARD.encode(format!(":{pat}"));
        Ok(Self {
            org: normalize_org(org)?,
            project: normalize_project(project)?,
            auth: format!("Basic {token}"),
            secret: pat.to_string(),
            http,
        })
    }

    /// Quita la credencial (y su forma codificada) de un texto que puede terminar en un error.
    fn scrub(&self, s: &str) -> String {
        let mut out = s.to_string();
        for secret in [&self.secret, &self.auth[6..].to_string()] {
            if secret.len() >= 4 {
                out = out.replace(secret.as_str(), "***");
            }
        }
        out
    }

    fn proj(&self) -> String {
        pct(&self.project, "")
    }

    fn project_not_found(&self) -> TrackerError {
        TrackerError::new("tracker.projectNotFound", vec![("project", self.project.clone())])
    }

    fn request(
        &self,
        method: &'static str,
        path: &str,
        params: &[(&str, String)],
        body: Option<(Json, &str)>,
        not_found: TrackerError,
    ) -> Result<Json> {
        let mut query: Vec<String> = params.iter().map(|(k, v)| format!("{k}={}", pct(v, ",/"))).collect();
        query.push(format!("api-version={API_VERSION}"));
        let url = format!("{}/{path}?{}", self.org, query.join("&"));
        let mut headers =
            vec![("Authorization", self.auth.clone()), ("Accept", "application/json".to_string())];
        let body = body.map(|(json, content_type)| {
            headers.push(("Content-Type", content_type.to_string()));
            json.to_string().into_bytes()
        });
        let req = HttpRequest { method, url, headers, body };
        let res = self.http.send(&req).map_err(|e| {
            TrackerError::new("tracker.network", vec![("org", self.org.clone()), ("message", self.scrub(&e))])
        })?;
        let detail = |max: usize| {
            let raw = String::from_utf8_lossy(&res.body).to_string();
            let msg = serde_json::from_str::<Json>(&raw)
                .ok()
                .and_then(|j| j["message"].as_str().map(String::from))
                .unwrap_or(raw);
            self.scrub(&msg.chars().take(max).collect::<String>()).trim().to_string()
        };
        match res.status {
            401 | 403 => Err(TrackerError::new(
                "tracker.auth",
                vec![("status", res.status.to_string()), ("detail", detail(200))],
            )),
            404 => {
                let mut e = not_found;
                e.params.push(("detail", detail(200)));
                Err(e)
            }
            s if !(200..300).contains(&s) => Err(TrackerError::new(
                "tracker.http",
                vec![("status", s.to_string()), ("detail", detail(400))],
            )),
            // Con una credencial inválida Azure a veces responde 203 con la página de login en vez de 401.
            s if s == 203 || !res.content_type.starts_with("application/json") => {
                Err(TrackerError::new("tracker.loginPage", vec![]))
            }
            _ if res.body.is_empty() => Ok(json!({})),
            _ => serde_json::from_slice(&res.body)
                .map_err(|e| TrackerError::new("tracker.badResponse", vec![("message", e.to_string())])),
        }
    }

    fn get(&self, path: &str, params: &[(&str, String)], not_found: TrackerError) -> Result<Json> {
        self.request("GET", path, params, None, not_found)
    }

    fn org_not_found(&self) -> TrackerError {
        TrackerError::new("tracker.orgNotFound", vec![("org", self.org.clone())])
    }

    fn item_not_found(id: &str) -> TrackerError {
        TrackerError::new("tracker.itemNotFound", vec![("id", id.to_string())])
    }

    fn parse_item(f: &Json, req: &FieldRequest) -> WorkItem {
        let assigned = &f["System.AssignedTo"];
        let mut it = WorkItem {
            state: text(&f["System.State"]),
            title: text(&f["System.Title"]),
            kind: text(&f["System.WorkItemType"]),
            assigned: assigned["displayName"].as_str().map_or_else(|| text(assigned), String::from),
            changed: text(&f["System.ChangedDate"]),
            ..Default::default()
        };
        if req.priority {
            it.priority =
                f[PRIORITY_FIELD].as_i64().or_else(|| f[PRIORITY_FIELD].as_f64().map(|p| p.round() as i64));
        }
        if let Some(due) = &req.due {
            it.due = text(&f[due.as_str()]);
        }
        if let Some(hours) = &req.hours {
            it.estimate = number(&f[hours.as_str()]);
        }
        if req.effort {
            it.completed = number(&f[COMPLETED_FIELD]);
        }
        if req.text {
            let desc =
                f[DESCRIPTION_FIELD].as_str().filter(|s| !s.is_empty()).or_else(|| f[REPRO_FIELD].as_str());
            it.description = Some(html_to_text(desc.unwrap_or_default()));
            it.criteria = Some(html_to_text(f[CRITERIA_FIELD].as_str().unwrap_or_default()));
        }
        for k in &req.extra {
            it.fields.insert(k.clone(), f[k.as_str()].clone());
        }
        it
    }
}

impl Tracker for AzureDevOps {
    fn states(&self) -> Result<Vec<TrackerState>> {
        let base = format!("{}/_apis/wit/workitemtypes", self.proj());
        let data = self.get(&base, &[], self.project_not_found())?;
        let mut out: Vec<TrackerState> = vec![];
        for t in data["value"].as_array().into_iter().flatten() {
            if t["isDisabled"].as_bool() == Some(true) {
                continue;
            }
            let name = text(&t["name"]);
            let states = match t["states"].as_array() {
                Some(s) => s.clone(),
                // Versiones viejas no traen los estados en el listado.
                None => {
                    let path = format!("{base}/{}/states", pct(&name, ""));
                    self.get(&path, &[], self.project_not_found())?["value"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default()
                }
            };
            for st in states {
                let sname = text(&st["name"]);
                if sname.is_empty() {
                    continue;
                }
                let entry = match out.iter().position(|s| s.name == sname) {
                    Some(i) => &mut out[i],
                    None => {
                        let category = text(&st["category"]);
                        out.push(TrackerState {
                            name: sname,
                            color: text(&st["color"]),
                            suggested: category_of(&category),
                            category,
                            types: vec![],
                        });
                        out.last_mut().expect("just pushed")
                    }
                };
                if !entry.types.contains(&name) {
                    entry.types.push(name.clone());
                }
            }
        }
        let rank = |c: &str| CATEGORY_ORDER.iter().position(|x| *x == c).unwrap_or(CATEGORY_ORDER.len());
        out.sort_by_key(|s| (rank(&s.category), s.name.to_lowercase()));
        Ok(out)
    }

    fn fields(&self) -> Result<Vec<FieldInfo>> {
        let data = self.get(&format!("{}/_apis/wit/fields", self.proj()), &[], self.project_not_found())?;
        Ok(data["value"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| {
                let reference = f["referenceName"].as_str()?.to_string();
                Some(FieldInfo {
                    name: f["name"].as_str().map_or_else(|| reference.clone(), String::from),
                    kind: text(&f["type"]),
                    reference,
                })
            })
            .collect())
    }

    fn work_items(&self, ids: &[String], req: &FieldRequest) -> Result<BTreeMap<String, WorkItem>> {
        let mut nums: Vec<u64> = ids.iter().filter_map(|i| i.trim().parse().ok()).collect();
        nums.sort_unstable();
        nums.dedup();
        let mut fields: Vec<&str> = BASE_FIELDS.to_vec();
        if req.priority {
            fields.push(PRIORITY_FIELD);
        }
        let optional = [req.due.as_deref(), req.hours.as_deref(), req.effort.then_some(COMPLETED_FIELD)];
        let text_fields =
            if req.text { vec![DESCRIPTION_FIELD, CRITERIA_FIELD, REPRO_FIELD] } else { vec![] };
        for f in optional.into_iter().flatten().chain(text_fields).chain(req.extra.iter().map(String::as_str))
        {
            if !fields.contains(&f) {
                fields.push(f);
            }
        }
        let fields = fields.join(",");
        let mut out = BTreeMap::new();
        for chunk in nums.chunks(BATCH) {
            let ids = chunk.iter().map(u64::to_string).collect::<Vec<_>>().join(",");
            let params = [("ids", ids), ("fields", fields.clone()), ("errorPolicy", "omit".to_string())];
            let data = self.get("_apis/wit/workitems", &params, self.org_not_found())?;
            // errorPolicy=omit deja null en los ids que no existen o no se pueden ver.
            for wi in data["value"].as_array().into_iter().flatten().filter(|w| w.is_object()) {
                let id = wi["id"].as_u64().map(|n| n.to_string()).unwrap_or_else(|| text(&wi["id"]));
                out.insert(id, Self::parse_item(&wi["fields"], req));
            }
        }
        Ok(out)
    }

    fn assigned_to_me(
        &self,
        due_field: &str,
        start: &str,
        end: &str,
        exclude: &[String],
    ) -> Result<Vec<String>> {
        let date_ok = |d: &str| wb_core::validate::parse_date(d).is_some();
        if !field_ref_ok(due_field) || !date_ok(start) || !date_ok(end) {
            return Err(TrackerError::new("tracker.badQuery", vec![]));
        }
        let mut wiql = format!(
            "SELECT [System.Id] FROM WorkItems WHERE [System.TeamProject] = @project AND [System.AssignedTo] = @Me \
             AND [{due_field}] >= '{start}' AND [{due_field}] < '{end}'"
        );
        if !exclude.is_empty() {
            let list: Vec<String> = exclude.iter().map(|s| format!("'{}'", s.replace('\'', "''"))).collect();
            wiql.push_str(&format!(" AND [System.State] NOT IN ({})", list.join(", ")));
        }
        wiql.push_str(&format!(" ORDER BY [{due_field}] ASC"));
        let data = self.request(
            "POST",
            &format!("{}/_apis/wit/wiql", self.proj()),
            &[("$top", "1000".to_string())],
            Some((json!({ "query": wiql }), "application/json")),
            self.project_not_found(),
        )?;
        Ok(data["workItems"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|w| w["id"].as_u64().map(|n| n.to_string()))
            .collect())
    }

    fn detail(&self, id: &str) -> Result<Detail> {
        if !is_id(id) {
            return Err(TrackerError::new("tracker.badId", vec![("id", id.to_string())]));
        }
        let fields = [
            "System.Title",
            "System.WorkItemType",
            "System.State",
            DESCRIPTION_FIELD,
            CRITERIA_FIELD,
            REPRO_FIELD,
        ];
        let data = self.get(
            &format!("_apis/wit/workitems/{id}"),
            &[("fields", fields.join(","))],
            Self::item_not_found(id),
        )?;
        let it = Self::parse_item(&data["fields"], &FieldRequest { text: true, ..Default::default() });
        Ok(Detail {
            id: data["id"].as_u64().map_or_else(|| id.to_string(), |n| n.to_string()),
            title: it.title,
            kind: it.kind,
            state: it.state,
            description: it.description.unwrap_or_default(),
            criteria: it.criteria.unwrap_or_default(),
        })
    }

    fn set_state(&self, id: &str, state: &str) -> Result<StateChange> {
        if !is_id(id) {
            return Err(TrackerError::new("tracker.badId", vec![("id", id.to_string())]));
        }
        let patch = json!([{ "op": "add", "path": "/fields/System.State", "value": state }]);
        let wi = self.request(
            "PATCH",
            &format!("_apis/wit/workitems/{id}"),
            &[],
            Some((patch, "application/json-patch+json")),
            Self::item_not_found(id),
        )?;
        let f = &wi["fields"];
        Ok(StateChange {
            state: f["System.State"].as_str().unwrap_or(state).to_string(),
            changed: text(&f["System.ChangedDate"]),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn org_urls() {
        assert_eq!(normalize_org(" miorg.visualstudio.com/ ").unwrap(), "https://miorg.visualstudio.com");
        assert_eq!(normalize_org("https://dev.azure.com/miorg").unwrap(), "https://dev.azure.com/miorg");
        for bad in [
            "",
            "http://miorg.visualstudio.com",
            "https://evil.com",
            "https://dev.azure.com",
            "https://x@dev.azure.com/o",
        ] {
            assert!(normalize_org(bad).is_err(), "{bad}");
        }
        assert_eq!(normalize_org("").unwrap_err().code, "tracker.noOrg");
        assert_eq!(normalize_project("  ").unwrap_err().code, "tracker.noProject");
        assert_eq!(pct("Mi Proyecto/á", ""), "Mi%20Proyecto%2F%C3%A1");
    }

    fn field(reference: &str, name: &str, kind: &str) -> FieldInfo {
        FieldInfo { reference: reference.into(), name: name.into(), kind: kind.into() }
    }

    #[test]
    fn field_catalog_and_hours_guess() {
        let fields = vec![
            field("System.ChangedDate", "Changed Date", "dateTime"),
            field("Microsoft.VSTS.Scheduling.DueDate", "Due Date", "dateTime"),
            field("Custom.FechaPactada", "Fecha pactada", "dateTime"),
            field("System.Id", "ID", "integer"),
            field(ESTIMATE_FIELD, "Original Estimate", "double"),
            field("Custom.AZ_DURACION_PACTADA", "Duracion pactada", "double"),
            field("Custom.Puntos", "Puntos", "integer"),
            field(PRIORITY_FIELD, "Priority", "integer"),
        ];
        let c = catalog(&fields);
        assert!(c.has_priority && !c.has_effort);
        let refs = |l: &[FieldInfo]| l.iter().map(|f| f.reference.clone()).collect::<Vec<_>>();
        assert_eq!(
            refs(&c.date_fields),
            ["Custom.FechaPactada", "System.ChangedDate", "Microsoft.VSTS.Scheduling.DueDate"]
        );
        assert_eq!(
            refs(&c.num_fields),
            ["Custom.AZ_DURACION_PACTADA", "Custom.Puntos", ESTIMATE_FIELD, PRIORITY_FIELD]
        );
        // «pactad» gana antes que «due date».
        assert_eq!(c.due_guess.as_deref(), Some("Custom.FechaPactada"));

        let cands = hours_candidates(&c.num_fields);
        assert_eq!(cands, ["Custom.AZ_DURACION_PACTADA", ESTIMATE_FIELD]);
        let item =
            |k: &str, v: f64| WorkItem { fields: [(k.to_string(), json!(v))].into(), ..Default::default() };
        let mut items = BTreeMap::new();
        assert_eq!(pick_hours(&cands, &items), ESTIMATE_FIELD);
        items.insert("1".into(), item("Custom.AZ_DURACION_PACTADA", 6.0));
        items.insert("2".into(), item(ESTIMATE_FIELD, 0.0));
        assert_eq!(pick_hours(&cands, &items), "Custom.AZ_DURACION_PACTADA");
        assert_eq!(pick_hours(&["Custom.Puntos".into()], &BTreeMap::new()), "");
    }
}
