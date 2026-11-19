mod common;

use common::*;
use wb_core::Workflow;
use wb_core::tasks::TaskSection;

fn section() -> TaskSection {
    let wf = Workflow::preset("classic-es").unwrap();
    TaskSection::new(&wf.tasks.heading, wf.tasks.max_len)
}

#[test]
fn task_operations() {
    let g = golden("tasks");
    let ts = section();
    for case in arr(&g["cases"]) {
        let body = s(&g["bodies"][s(&case["body"])]);
        let args = case.get("args").map(|a| arr(a).clone()).unwrap_or_default();
        let expect = case.get("expect").map(s);
        let ctx = format!("{} {} {:?} expect={expect:?}", case["body"], case["op"], args);
        let idx = || args[0].as_i64().unwrap();
        match s(&case["op"]) {
            "parse" => assert_outcome(&Ok(ts.parse(body)), case, &ctx),
            "add" => assert_outcome(&Ok(ts.add(body, s(&args[0]))), case, &ctx),
            "close" => assert_outcome(&Ok(ts.close_all(body)), case, &ctx),
            "clear_done" => assert_outcome(&Ok(ts.clear_done(body)), case, &ctx),
            "set" => assert_outcome(&ts.set(body, idx(), args[1].as_bool().unwrap(), expect), case, &ctx),
            "remove" => assert_outcome(&ts.remove(body, idx(), expect), case, &ctx),
            "edit" => assert_outcome(&ts.edit(body, idx(), s(&args[1]), expect), case, &ctx),
            other => panic!("unknown op {other}"),
        }
    }
}

#[test]
fn clean() {
    let ts = section();
    for case in arr(&golden("tasks")["clean"]) {
        assert_outcome(&ts.clean(&py_str(&case["text"])), case, &case["text"].to_string());
    }
}
