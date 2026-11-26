//! Cliente de Azure DevOps contra respuestas fijas (formato real de la API 7.0), sin red.

use std::sync::{Arc, Mutex};

use serde_json::{Value as Json, json};
use wb_core::StateCategory;
use wb_tracker::azure::AzureDevOps;
use wb_tracker::{FieldRequest, HttpRequest, HttpResponse, Tracker, Transport};

const PAT: &str = "s3cr3t-pat-value";

/// Responde según el método y un fragmento de la URL; guarda cada pedido.
#[derive(Default)]
struct Fake {
    routes: Vec<(&'static str, &'static str, u16, &'static str, String)>,
    log: Mutex<Vec<HttpRequest>>,
}

impl Fake {
    fn on(mut self, method: &'static str, path: &'static str, status: u16, body: Json) -> Self {
        self.routes.push((method, path, status, "application/json; charset=utf-8", body.to_string()));
        self
    }

    fn raw(
        mut self,
        method: &'static str,
        path: &'static str,
        status: u16,
        ct: &'static str,
        body: &str,
    ) -> Self {
        self.routes.push((method, path, status, ct, body.to_string()));
        self
    }

    fn requests(&self) -> Vec<HttpRequest> {
        self.log.lock().unwrap().clone()
    }
}

impl Transport for Fake {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse, String> {
        self.log.lock().unwrap().push(req.clone());
        let path = req.url.split('?').next().unwrap();
        self.routes
            .iter()
            .find(|(m, p, ..)| *m == req.method && path.ends_with(p))
            .map(|(_, _, status, ct, body)| HttpResponse {
                status: *status,
                content_type: ct.to_string(),
                body: body.as_bytes().to_vec(),
            })
            .ok_or_else(|| format!("no route for {} {}", req.method, req.url))
    }
}

fn client(fake: Fake) -> (AzureDevOps, Arc<Fake>) {
    let fake = Arc::new(fake);
    (AzureDevOps::new("miorg.visualstudio.com", "Mi Proyecto", PAT, fake.clone()).unwrap(), fake)
}

fn types_response() -> Json {
    json!({"count": 3, "value": [
        {"name": "Bug", "isDisabled": false, "states": [
            {"name": "New", "color": "b2b2b2", "category": "Proposed"},
            {"name": "Active", "color": "007acc", "category": "InProgress"},
            {"name": "Resolved", "color": "ff9d00", "category": "Resolved"},
            {"name": "Closed", "color": "339933", "category": "Completed"}]},
        {"name": "User Story", "isDisabled": false, "states": [
            {"name": "New", "color": "b2b2b2", "category": "Proposed"},
            {"name": "Active", "color": "007acc", "category": "InProgress"},
            {"name": "Closed", "color": "339933", "category": "Completed"},
            {"name": "Removed", "color": "ffffff", "category": "Removed"}]},
        {"name": "Old", "isDisabled": true, "states": [{"name": "Zombie", "category": "Proposed"}]},
        {"name": "Task", "isDisabled": false}
    ]})
}

#[test]
fn states_are_aggregated_by_name_and_sorted_by_category() {
    let (c, fake) = client(
        Fake::default().on("GET", "/Mi%20Proyecto/_apis/wit/workitemtypes", 200, types_response()).on(
            "GET",
            "/workitemtypes/Task/states",
            200,
            json!({"value": [
                {"name": "To Do", "color": "b2b2b2", "category": "Proposed"},
                {"name": "Done", "color": "339933", "category": "Completed"}]}),
        ),
    );
    let states = c.states().unwrap();
    let names: Vec<&str> = states.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["New", "To Do", "Active", "Resolved", "Closed", "Done", "Removed"]);
    let new = &states[0];
    assert_eq!(new.types, ["Bug", "User Story"]);
    assert_eq!(new.suggested, Some(StateCategory::Backlog));
    assert_eq!(states[3].suggested, Some(StateCategory::Review));
    assert_eq!(states.last().unwrap().suggested, Some(StateCategory::Cancelled));
    let reqs = fake.requests();
    assert!(reqs.iter().all(|r| r.method == "GET" && r.body.is_none()));
    assert!(reqs[0].url.ends_with("?api-version=7.0"), "{}", reqs[0].url);
    // Basic con usuario vacío y el PAT.
    assert_eq!(reqs[0].header("authorization").unwrap(), "Basic OnMzY3IzdC1wYXQtdmFsdWU=");
}

#[test]
fn fields_and_work_items_in_batches() {
    let ids: Vec<String> = (1..=250).map(|i| (1000 + i).to_string()).chain(["abc".to_string()]).collect();
    let first = json!({"count": 2, "value": [
        {"id": 1001, "fields": {
            "System.Id": 1001, "System.State": "Active", "System.Title": "Pagos",
            "System.WorkItemType": "Bug",
            "System.AssignedTo": {"displayName": "Ana Pérez", "uniqueName": "ana@x.com"},
            "System.ChangedDate": "2026-11-02T10:00:00.123Z",
            "Microsoft.VSTS.Common.Priority": 2,
            "Custom.Pactada": "2026-11-25T03:00:00Z",
            "Custom.Horas": 6.5,
            "Microsoft.VSTS.Scheduling.CompletedWork": 1.5}},
        null,
        {"id": 1003, "fields": {"System.State": "New", "System.Title": "Otro", "System.WorkItemType": "User Story",
            "System.AssignedTo": "Juan"}}
    ]});
    let (c, fake) = client(
        Fake::default()
            .on(
                "GET",
                "/_apis/wit/fields",
                200,
                json!({"value": [
                {"referenceName": "Custom.Pactada", "name": "Fecha pactada", "type": "dateTime"},
                {"referenceName": "Custom.Horas", "name": "Horas", "type": "double"}]}),
            )
            .on("GET", "/_apis/wit/workitems", 200, first),
    );
    let fields = c.fields().unwrap();
    assert_eq!(fields[0].reference, "Custom.Pactada");
    assert_eq!(fields[1].kind, "double");

    let req = FieldRequest {
        priority: true,
        due: Some("Custom.Pactada".into()),
        hours: Some("Custom.Horas".into()),
        effort: true,
        ..Default::default()
    };
    let items = c.work_items(&ids, &req).unwrap();
    assert_eq!(items.len(), 2);
    let a = &items["1001"];
    assert_eq!(a.state, "Active");
    assert_eq!(a.assigned, "Ana Pérez");
    assert_eq!(a.priority, Some(2));
    assert_eq!(a.due, "2026-11-25T03:00:00Z");
    assert_eq!(a.estimate, Some(6.5));
    assert_eq!(a.completed, Some(1.5));
    assert_eq!(items["1003"].assigned, "Juan");
    assert_eq!(items["1003"].priority, None);

    let reqs: Vec<HttpRequest> =
        fake.requests().into_iter().filter(|r| r.url.contains("workitems?")).collect();
    assert_eq!(reqs.len(), 2, "250 ids = 2 lotes");
    assert!(reqs[0].url.contains("ids=1001,1002,"), "{}", reqs[0].url);
    assert!(reqs[0].url.contains("errorPolicy=omit"));
    assert!(reqs[0].url.contains(
        "fields=System.Id,System.State,System.Title,System.WorkItemType,System.AssignedTo,System.ChangedDate,\
         Microsoft.VSTS.Common.Priority,Custom.Pactada,Custom.Horas,Microsoft.VSTS.Scheduling.CompletedWork&"
    ));
    assert_eq!(reqs[1].url.split("ids=").nth(1).unwrap().split('&').next().unwrap().split(',').count(), 50);
}

#[test]
fn errors_never_carry_the_secret() {
    let (c, _) = client(Fake::default().on(
        "GET",
        "/_apis/wit/fields",
        401,
        json!({"message": format!("TF400813: token {PAT} is not authorized")}),
    ));
    let e = c.fields().unwrap_err();
    assert_eq!(e.code, "tracker.auth");
    assert!(!format!("{e}").contains(PAT), "{e}");
    assert!(format!("{e}").contains("TF400813"));

    let (c, _) =
        client(Fake::default().raw("GET", "/_apis/wit/fields", 203, "text/html", "<html>login</html>"));
    assert_eq!(c.fields().unwrap_err().code, "tracker.loginPage");

    let (c, _) = client(Fake::default().on("GET", "/_apis/wit/fields", 404, json!({"message": "TF200016"})));
    let e = c.fields().unwrap_err();
    assert_eq!(e.code, "tracker.projectNotFound");
    assert_eq!(e.params[0], ("project", "Mi Proyecto".to_string()));
    assert_eq!(e.params[1], ("org", "https://miorg.visualstudio.com".to_string()));

    let (c, _) = client(Fake::default().on("GET", "/_apis/wit/fields", 500, json!({"message": "boom"})));
    assert_eq!(
        c.fields().unwrap_err().params,
        vec![("source", "Azure DevOps".into()), ("status", "500".into()), ("detail", "boom".into())]
    );

    // Sin respuesta: error de red con la organización (nunca el header de autorización).
    let (c, _) = client(Fake::default());
    let e = c.fields().unwrap_err();
    assert_eq!(e.code, "tracker.network");
    assert!(!format!("{e}").contains("OnMzY3"));
    assert!(AzureDevOps::new("miorg.visualstudio.com", "P", "  ", Arc::new(Fake::default())).is_err());
}

#[test]
fn assigned_query_is_a_read_only_wiql_post() {
    let (c, fake) = client(Fake::default().on(
        "POST",
        "/Mi%20Proyecto/_apis/wit/wiql",
        200,
        json!({"queryType": "flat", "workItems": [{"id": 7, "url": "x"}, {"id": 3, "url": "y"}]}),
    ));
    let ids = c
        .assigned_to_me("Custom.Pactada", "2026-11-20", "2026-12-20", &["Closed".into(), "Won't fix".into()])
        .unwrap();
    assert_eq!(ids, ["7", "3"]);
    let req = &fake.requests()[0];
    assert!(req.url.contains("$top=1000"));
    let body: Json = serde_json::from_slice(req.body.as_ref().unwrap()).unwrap();
    assert_eq!(
        body["query"],
        "SELECT [System.Id] FROM WorkItems WHERE [System.TeamProject] = @project AND [System.AssignedTo] = @Me \
         AND [Custom.Pactada] >= '2026-11-20' AND [Custom.Pactada] < '2026-12-20' \
         AND [System.State] NOT IN ('Closed', 'Won''t fix') ORDER BY [Custom.Pactada] ASC"
    );
    // Un campo o una fecha raros no llegan a la consulta.
    assert_eq!(
        c.assigned_to_me("X] OR 1=1 --", "2026-11-20", "2026-12-20", &[]).unwrap_err().code,
        "tracker.badQuery"
    );
    assert_eq!(
        c.assigned_to_me("Custom.P", "2026-11-2", "2026-12-20", &[]).unwrap_err().code,
        "tracker.badQuery"
    );
}

#[test]
fn detail_converts_html_and_set_state_patches_only_the_state() {
    let (c, fake) = client(
        Fake::default()
            .on(
                "GET",
                "/_apis/wit/workitems/123456",
                200,
                json!({"id": 123456, "fields": {
                "System.Title": "Pagos duplicados", "System.WorkItemType": "Bug", "System.State": "Active",
                "System.Description": "",
                "Microsoft.VSTS.TCM.ReproSteps": "<div>Paso&nbsp;1</div><ul><li>a</li></ul>",
                "Microsoft.VSTS.Common.AcceptanceCriteria": "<p>Que no se dupliquen</p>"}}),
            )
            .on(
                "GET",
                "/_apis/wit/workitems/9",
                404,
                json!({"message": "TF401232: Work item 9 does not exist"}),
            )
            .on(
                "PATCH",
                "/_apis/wit/workitems/123456",
                200,
                json!({"id": 123456, "fields": {
                "System.State": "Resolved", "System.ChangedDate": "2026-11-19T12:00:00Z"}}),
            ),
    );
    let d = c.detail("123456").unwrap();
    assert_eq!(d.title, "Pagos duplicados");
    assert_eq!(d.description, "Paso 1\n- a");
    assert_eq!(d.criteria, "Que no se dupliquen");
    assert_eq!(c.detail("9").unwrap_err().code, "tracker.itemNotFound");
    assert_eq!(c.detail("9; DROP").unwrap_err().code, "tracker.badId");

    let res = c.set_state("123456", "Resolved").unwrap();
    assert_eq!((res.state.as_str(), res.changed.as_str()), ("Resolved", "2026-11-19T12:00:00Z"));
    let patch = fake.requests().into_iter().find(|r| r.method == "PATCH").unwrap();
    assert_eq!(patch.header("content-type").unwrap(), "application/json-patch+json");
    let body: Json = serde_json::from_slice(patch.body.as_ref().unwrap()).unwrap();
    assert_eq!(body, json!([{"op": "add", "path": "/fields/System.State", "value": "Resolved"}]));
}
