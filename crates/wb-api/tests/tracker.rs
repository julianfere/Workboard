//! Comandos `tracker_*` de punta a punta contra un Azure DevOps simulado (sin red) y un almacén de
//! secretos en memoria.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{Value as Json, json};
use wb_api::Api;
use wb_platform::secrets::{MemorySecrets, SecretStore};
use wb_tracker::{HttpRequest, HttpResponse, Transport};

const PAT: &str = "read-pat-0123456789";
const WRITE_PAT: &str = "write-pat-9876543210";

#[derive(Default)]
struct Fake {
    routes: Vec<(&'static str, &'static str, Json)>,
    log: Mutex<Vec<HttpRequest>>,
}

impl Transport for Fake {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse, String> {
        self.log.lock().unwrap().push(req.clone());
        let path = req.url.split('?').next().unwrap();
        let (_, _, body) = self
            .routes
            .iter()
            .find(|(m, p, _)| *m == req.method && path.ends_with(p))
            .ok_or_else(|| format!("no route for {} {}", req.method, req.url))?;
        Ok(HttpResponse {
            status: 200,
            content_type: "application/json".into(),
            body: body.to_string().into_bytes(),
        })
    }
}

fn state(name: &str, category: &str) -> Json {
    json!({"name": name, "color": "b2b2b2", "category": category})
}

fn wi(id: u64, st: &str, kind: &str, extra: Json) -> Json {
    let mut fields = json!({"System.State": st, "System.Title": format!("Título {id}"), "System.WorkItemType": kind,
        "System.AssignedTo": {"displayName": "Ana"}, "System.ChangedDate": "2026-11-01T10:00:00Z",
        "System.Description": "<div>Descripción de <b>prueba</b></div>",
        "Microsoft.VSTS.Common.AcceptanceCriteria": "<ul><li>uno</li><li>dos</li></ul>"});
    for (k, v) in extra.as_object().unwrap() {
        fields[k] = v.clone();
    }
    json!({"id": id, "fields": fields})
}

fn azure() -> Fake {
    let states = json!({"value": [
        {"name": "Bug", "states": [state("New", "Proposed"), state("Active", "InProgress"), state("Resolved", "Resolved"), state("Closed", "Completed")]},
        {"name": "User Story", "states": [state("New", "Proposed"), state("Active", "InProgress"), state("Closed", "Completed"), state("Removed", "Removed")]},
        {"name": "Task", "states": [state("New", "Proposed"), state("Active", "InProgress"), state("Closed", "Completed")]},
    ]});
    let fields = json!({"value": [
        {"referenceName": "System.ChangedDate", "name": "Changed Date", "type": "dateTime"},
        {"referenceName": "Custom.Pactada", "name": "Fecha pactada", "type": "dateTime"},
        {"referenceName": "Custom.Horas", "name": "Horas pactadas", "type": "double"},
        {"referenceName": "Microsoft.VSTS.Common.Priority", "name": "Priority", "type": "integer"},
    ]});
    let p = |prio: i64, due: &str, h: f64| json!({"Microsoft.VSTS.Common.Priority": prio, "Custom.Pactada": due, "Custom.Horas": h});
    let items = json!({"value": [
        wi(111111, "Active", "User Story", p(2, "2026-11-25T03:00:00Z", 6.5)),
        wi(222222, "Resolved", "Bug", p(1, "2026-12-01T03:00:00Z", 3.0)),
        wi(333333, "Closed", "Bug", json!({})),
        wi(444444, "Active", "Task", json!({})),
        wi(555555, "Zombie", "Task", json!({})),
        wi(666666, "Closed", "Bug", json!({})),
        wi(777777, "New", "Task", json!({})),
        null,
    ]});
    Fake {
        routes: vec![
            ("GET", "/Proyecto%20X/_apis/wit/workitemtypes", states),
            ("GET", "/Proyecto%20X/_apis/wit/fields", fields),
            ("GET", "/_apis/wit/workitems", items),
            ("POST", "/Proyecto%20X/_apis/wit/wiql", json!({"workItems": [{"id": 888888}, {"id": 111111}]})),
            ("GET", "/_apis/wit/workitems/123456", wi(123456, "New", "Bug", json!({}))),
            (
                "PATCH",
                "/_apis/wit/workitems/222222",
                json!({"id": 222222, "fields": {"System.State": "Closed", "System.ChangedDate": "2026-11-19T12:00:00Z"}}),
            ),
        ],
        log: Mutex::default(),
    }
}

fn fixture(tag: &str) -> (PathBuf, PathBuf) {
    let base = std::env::temp_dir().join(format!("wb-api-tracker-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/fixtures/work");
    fn copy(src: &Path, dest: &Path) {
        std::fs::create_dir_all(dest).unwrap();
        for e in std::fs::read_dir(src).unwrap().flatten() {
            if e.path().is_dir() {
                copy(&e.path(), &dest.join(e.file_name()));
            } else {
                std::fs::copy(e.path(), dest.join(e.file_name())).unwrap();
            }
        }
    }
    copy(&src, &base.join("work"));
    (base.join("work"), base)
}

struct Env {
    api: Api,
    http: Arc<Fake>,
    secrets: Arc<MemorySecrets>,
    root: PathBuf,
    base: PathBuf,
}

fn setup(tag: &str) -> Env {
    let (root, base) = fixture(tag);
    let api = Api::new(base.join("cfg"));
    let http = Arc::new(azure());
    let secrets = Arc::new(MemorySecrets::default());
    api.set_tracker_backend(secrets.clone(), http.clone());
    api.dispatch("set_settings", &json!({"settings": {"workflow": "classic-es", "lang": "es"}})).unwrap();
    api.dispatch("open_root", &json!({"path": root})).unwrap();
    Env { api, http, secrets, root, base }
}

impl Env {
    fn call(&self, cmd: &str, args: Json) -> Json {
        self.api.dispatch(cmd, &args).unwrap_or_else(|e| panic!("{cmd}: {e:?}"))
    }

    fn connect(&self) -> Json {
        self.call(
            "tracker_connect",
            json!({"org": "miorg.visualstudio.com", "project": "Proyecto X", "secret": PAT}),
        );
        self.call("tracker_fetch", json!({}))
    }

    fn methods(&self) -> Vec<&'static str> {
        self.http.log.lock().unwrap().iter().map(|r| r.method).collect()
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn file_text(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

#[test]
fn connect_keeps_the_secret_out_of_files() {
    let env = setup("connect");
    let cfg = env.call("tracker_config", json!({}));
    assert_eq!(cfg["connected"], false);
    let err = env
        .api
        .dispatch("tracker_connect", &json!({"org": "miorg.visualstudio.com", "project": "P"}))
        .unwrap_err();
    assert_eq!(err.code, "tracker.noSecretFor");
    let err = env
        .api
        .dispatch("tracker_connect", &json!({"org": "https://evil.com", "project": "P", "secret": "x"}))
        .unwrap_err();
    assert_eq!(err.code, "tracker.badOrg");
    assert_eq!(env.api.dispatch("tracker_sync", &json!({})).unwrap_err().code, "tracker.notConnected");

    let cfg = env.connect();
    assert_eq!(cfg["connected"], true);
    assert_eq!(cfg["org"], "https://miorg.visualstudio.com");
    assert_eq!(cfg["hasSecret"], true);
    assert_eq!(cfg["hasWriteSecret"], false);
    assert_eq!(
        cfg["suggested"],
        json!({"New": "pendiente", "Active": "en-curso", "Resolved": "codeado", "Closed": "prod", "Removed": "descartado"})
    );
    assert_eq!(cfg["states"][0]["types"], json!(["Bug", "User Story", "Task"]));
    assert_eq!(cfg["dueField"], "Custom.Pactada", "se autoelige por «pactad»");
    assert_eq!(cfg["hoursField"], "", "se elige en el primer sync");
    assert_eq!(cfg["localTypes"], json!({"Bug": "bug", "Task": "task", "User Story": "us"}));
    assert_eq!(
        env.secrets.get("azure-devops:read:https://miorg.visualstudio.com").unwrap().as_deref(),
        Some(PAT)
    );
    let cfg_dir = env.base.join("cfg");
    for f in ["trackers.json", "settings.json"] {
        assert!(!file_text(&cfg_dir.join(f)).contains(PAT), "{f}");
    }
    // Reconectar sin token usa el guardado; desconectar conserva el token y el mapeo.
    env.call("tracker_connect", json!({"org": "miorg.visualstudio.com", "project": "Proyecto X"}));
    let cfg = env.call("tracker_disconnect", json!({}));
    assert_eq!(cfg["connected"], false);
    assert!(env.secrets.get("azure-devops:read:https://miorg.visualstudio.com").unwrap().is_some());
    let cfg = env.call("tracker_connect", json!({"org": "miorg.visualstudio.com", "project": "Proyecto X"}));
    assert_eq!(cfg["states"].as_array().unwrap().len(), 5);
    let cfg = env.call("tracker_clear_secret", json!({}));
    assert_eq!(cfg["hasSecret"], false);
    assert_eq!(env.methods().iter().filter(|m| **m != "GET").count(), 0, "conectar solo lee");
}

#[test]
fn sync_preview_and_apply_only_touch_status_and_type() {
    let env = setup("sync");
    env.connect();
    let suggested = env.call("tracker_config", json!({}))["suggested"].clone();
    let cfg = env.call("tracker_set_map", json!({"map": suggested}));
    assert_eq!(cfg["mapSaved"], true);
    assert_eq!(
        env.call("tracker_set_field", json!({"which": "due", "ref": "Custom.Pactada"}))["dueField"],
        "Custom.Pactada"
    );
    assert_eq!(
        env.api.dispatch("tracker_set_field", &json!({"which": "due", "ref": "Nope"})).unwrap_err().code,
        "tracker.unknownField"
    );

    let cfg = env.call("tracker_sync", json!({}));
    assert_eq!(cfg["hoursField"], "Custom.Horas", "el campo de horas con más work items cargados");
    let cache = &cfg["cache"];
    assert_eq!(cache["source"], "Azure DevOps");
    assert!(cache["syncedAt"].as_f64().unwrap() > 0.0);
    // El formato que lee el MCP: prioridad, fecha y horas por id.
    let raw: Json = serde_json::from_str(&file_text(&env.base.join("cfg/tracker_cache.json"))).unwrap();
    let key = wb_core::records::RunLog::root_key(&env.root);
    assert_eq!(raw[&key]["items"]["111111"]["priority"], 2);
    assert_eq!(raw[&key]["items"]["111111"]["due"], "2026-11-25T03:00:00Z");
    assert_eq!(raw[&key]["items"]["111111"]["estimate"], 6.5);
    assert!(raw[&key]["items"]["111111"].get("description").is_none(), "la descripción no va a la caché");
    assert!(raw[&key]["requested"].as_array().unwrap().contains(&json!("777777")));

    let p = env.call("tracker_preview", json!({"includeDiffer": false}));
    let rows = p["rows"].as_array().unwrap();
    let row = |name: &str| rows.iter().find(|r| r["name"] == name).cloned().unwrap_or(Json::Null);
    let fill = row("wi777777-sin-readme");
    assert_eq!(
        (fill["kind"].as_str(), fill["status"].as_str(), fill["type"].as_str()),
        (Some("fill"), Some("pendiente"), Some("task"))
    );
    assert_eq!(fill["on"], true);
    // Con otro estado local: sin incluirlos solo se completa el tipo que falta.
    assert_eq!(row("wi666666-plan-pendiente")["kind"], "type");
    assert_eq!(row("wi666666-plan-pendiente")["status"], "");
    assert_eq!(row("wi111111-login-con-sso"), Json::Null, "ya coincide y tiene tipo");
    assert_eq!(p["equal"], 1);
    assert_eq!(
        p["unmapped"],
        json!([{"state": "Zombie", "items": ["arreglo-sin-wi", "grp-checkout/wi555555-envio"]}])
    );

    let p = env.call("tracker_preview", json!({"includeDiffer": true}));
    let rows = p["rows"].as_array().unwrap();
    let row = |name: &str| rows.iter().find(|r| r["name"] == name).cloned().unwrap();
    let back = row("wi222222-333333-pagos");
    assert_eq!(
        (back["kind"].as_str(), back["status"].as_str()),
        (Some("back"), Some("codeado")),
        "manda el menos avanzado"
    );
    assert_eq!(back["on"], false, "retroceder queda destildado");
    let adv = row("wi666666-plan-pendiente");
    assert_eq!(
        (adv["kind"].as_str(), adv["status"].as_str(), adv["on"].as_bool()),
        (Some("differ"), Some("prod"), Some(true))
    );
    let order: Vec<&str> = rows.iter().map(|r| r["kind"].as_str().unwrap()).collect();
    let mut sorted = order.clone();
    sorted.sort_by_key(|k| ["fill", "type", "differ", "back"].iter().position(|x| x == k));
    assert_eq!(order, sorted);
    let only =
        env.call("tracker_preview", json!({"includeDiffer": true, "only": ["wi666666-plan-pendiente"]}));
    assert_eq!(only["rows"].as_array().unwrap().len(), 1);

    let res = env.call(
        "tracker_apply",
        json!({"changes": [
            {"name": "wi777777-sin-readme", "status": "pendiente", "type": "task"},
            {"name": "wi666666-plan-pendiente", "status": "prod"},
            {"name": "wi111111-login-con-sso"},
            {"name": "wi222222-333333-pagos", "status": "nope"},
        ]}),
    );
    assert_eq!(res["done"], json!(["wi777777-sin-readme", "wi666666-plan-pendiente"]));
    assert_eq!(res["errors"][0]["error"]["code"], "tracker.nothingToApply");
    assert_eq!(res["errors"][1]["error"]["code"], "core.field.oneOf");
    let readme = file_text(&env.root.join("wi777777-sin-readme/README.md"));
    assert!(readme.contains("estado: pendiente") && readme.contains("tipo: task"), "{readme}");
    assert!(file_text(&env.root.join("wi666666-plan-pendiente/README.md")).contains("estado: prod"));
    assert!(env.methods().iter().all(|m| *m == "GET"), "el sync solo hace GET");
}

#[test]
fn assigned_import_and_new_item_from_the_tracker() {
    let env = setup("assigned");
    env.connect();
    let list = env.call("tracker_assigned", json!({"period": "2026-11"}));
    assert_eq!((list["start"].as_str(), list["end"].as_str()), (Some("2026-11-20"), Some("2026-12-19")));
    assert_eq!(list["field"], "Custom.Pactada");
    // 888888 no vuelve en el GET (no existe o no se ve): se omite.
    let items = list["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], "111111");
    assert_eq!(items[0]["localType"], "us");
    assert_eq!(items[0]["existing"], json!(["wi111111-copia-vieja", "wi111111-login-con-sso"]));
    let wiql = env.http.log.lock().unwrap().iter().find(|r| r.method == "POST").cloned().unwrap();
    let q: Json = serde_json::from_slice(wiql.body.as_ref().unwrap()).unwrap();
    let q = q["query"].as_str().unwrap();
    assert!(q.contains("[Custom.Pactada] >= '2026-11-20' AND [Custom.Pactada] < '2026-12-20'"), "{q}");
    assert!(q.contains("NOT IN ('Closed', 'Removed')"), "{q}");
    assert_eq!(
        env.api.dispatch("tracker_assigned", &json!({"period": "2026-13"})).unwrap_err().code,
        "tracker.badPeriod"
    );

    let res = env.call("tracker_import", json!({"ids": ["111111", "#123456"]}));
    assert_eq!(res["skipped"], json!(["111111"]));
    assert_eq!(res["errors"][0]["id"], "123456", "no vino en el lote");
    assert_eq!(res["errors"][0]["error"]["code"], "tracker.itemMissing");
    assert_eq!(
        env.api.dispatch("tracker_import", &json!({"ids": ["abc"]})).unwrap_err().code,
        "tracker.pickItems"
    );

    // Nuevo item: trae los datos y crea la carpeta con descripción y criterios del tracker.
    let d = env.call("tracker_item", json!({"id": "#123456"}));
    assert_eq!(d["fromTracker"], true);
    assert_eq!(d["title"], "Título 123456");
    assert_eq!(d["localType"], "bug");
    assert_eq!(d["folder"], "wi123456-titulo-123456");
    assert_eq!(d["description"], "Descripción de prueba");
    let it = env.call(
        "tracker_create_item",
        json!({"id": "123456", "title": d["title"], "type": d["localType"], "folder": d["folder"],
            "description": d["description"], "criteria": d["criteria"], "trackerType": d["type"], "trackerState": d["state"]}),
    );
    assert_eq!(it["item"]["name"], "wi123456-titulo-123456");
    assert_eq!(it["session"], Json::Null);
    let readme = file_text(&env.root.join("wi123456-titulo-123456/README.md"));
    // Como la v1 (`_plan_wi_create`): origen «Azure: estado», plan con el texto de relleno y pendientes.
    assert_eq!(
        readme,
        "---\nestado: pendiente\nwi: 123456\ntipo: bug\n---\n# Título 123456\n\n> Bug #123456 · Azure: New\n\n\
         ## Descripción\n\nDescripción de prueba\n\n## Criterios de aceptación\n\n- uno\n- dos\n\n\
         ## Plan\n\n*(lo completa la sesión de planificación)*\n\n## Pendientes\n\n"
    );
    // Sin conexión: vuelve vacío para completarlo a mano.
    env.call("tracker_disconnect", json!({}));
    let d = env.call("tracker_item", json!({"id": "999999"}));
    assert_eq!((d["fromTracker"].as_bool(), d["folder"].as_str()), (Some(false), Some("wi999999")));
    assert_eq!(env.api.dispatch("tracker_item", &json!({"id": "12"})).unwrap_err().code, "tracker.badId");
}

#[test]
fn push_states_writes_only_with_the_write_secret() {
    let env = setup("push");
    env.connect();
    env.call("tracker_set_map", json!({"map": {"Closed": "prod"}}));
    env.call("tracker_sync", json!({}));
    let err = env
        .api
        .dispatch("tracker_push_states", &json!({"changes": [{"id": "222222", "state": "Closed"}]}))
        .unwrap_err();
    assert_eq!(err.code, "tracker.noWriteSecret");
    assert!(!env.methods().contains(&"PATCH"));

    let res = env.call(
        "tracker_push_states",
        json!({"changes": [{"id": "222222", "state": "Closed"}, {"id": "222222", "state": "Inventado"}, {"id": "x", "state": "Closed"}],
            "secret": WRITE_PAT}),
    );
    assert_eq!(res["done"], json!([{"id": "222222", "state": "Closed"}]));
    assert_eq!(res["errors"][0]["error"]["code"], "tracker.unknownState");
    assert_eq!(res["errors"][1]["error"]["code"], "tracker.badId");
    let patch = env.http.log.lock().unwrap().iter().find(|r| r.method == "PATCH").cloned().unwrap();
    use base64::Engine;
    let expected = base64::engine::general_purpose::STANDARD.encode(format!(":{WRITE_PAT}"));
    assert_eq!(
        patch.header("authorization").unwrap(),
        format!("Basic {expected}"),
        "con la credencial de escritura"
    );
    let cfg = env.call("tracker_config", json!({}));
    assert_eq!(cfg["hasWriteSecret"], true);
    assert_eq!(cfg["cache"]["items"]["222222"]["state"], "Closed");
    assert_eq!(cfg["cache"]["items"]["222222"]["changed"], "2026-11-19T12:00:00Z");
    for f in ["trackers.json", "tracker_cache.json"] {
        assert!(!file_text(&env.base.join("cfg").join(f)).contains(WRITE_PAT), "{f}");
    }
    assert_eq!(env.api.dispatch("tracker_open", &json!({"id": "x"})).unwrap_err().code, "tracker.badId");
}

#[test]
fn due_field_guess_is_retried_on_every_sync_like_v1() {
    let (root, base) = fixture("dueguess");
    let api = Api::new(base.join("cfg"));
    let mut fake = azure();
    // Sin un campo «pactad…»: «Due Date» no se adivina (la v1 no lo hacía).
    fake.routes[1].2 = json!({"value": [
        {"referenceName": "Microsoft.VSTS.Scheduling.DueDate", "name": "Due Date", "type": "dateTime"},
        {"referenceName": "Custom.Horas", "name": "Horas pactadas", "type": "double"},
    ]});
    let http = Arc::new(fake);
    api.set_tracker_backend(Arc::new(MemorySecrets::default()), http.clone());
    api.dispatch("set_settings", &json!({"settings": {"workflow": "classic-es", "lang": "es"}})).unwrap();
    api.dispatch("open_root", &json!({"path": root})).unwrap();
    let env = Env { api, http, secrets: Arc::new(MemorySecrets::default()), root, base };
    let cfg = env.connect();
    assert_eq!(cfg["dueField"], "");
    let field_reads =
        || env.http.log.lock().unwrap().iter().filter(|r| r.url.contains("/_apis/wit/fields")).count();
    assert_eq!(field_reads(), 1);
    env.call("tracker_sync", json!({}));
    env.call("tracker_sync", json!({}));
    assert_eq!(field_reads(), 3, "sin fecha adivinada, cada sync vuelve a intentar");
    // Elegir «ninguna» corta el reintento.
    env.call("tracker_set_field", json!({"which": "due", "ref": ""}));
    env.call("tracker_sync", json!({}));
    assert_eq!(field_reads(), 3);
}
