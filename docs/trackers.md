# Trackers

Workboard can link a work folder to an external tracker to read the state, title, type, priority, agreed
date and estimate of each item's work item. Azure DevOps is the only tracker today; the code is behind a
trait so others can be added (see [Adding a tracker](#adding-a-tracker)).

The rules:

- **Read-only by default.** Syncing only reads. Writing to the tracker is a separate action
  (**Change state in Azure DevOps**), done with a separate write token and only when you ask for it.
- **Tokens live in the system credential store** (Windows Credential Manager, macOS Keychain, Secret
  Service on Linux), never in Workboard's files, logs or error messages.
- **In the README, only the status and the type are written** (when you apply a sync), plus the new folder
  and README when you create an item from a work item.

## Azure DevOps setup

In **Settings → Tracker**, with the work folder open:

1. **Organization**: `https://dev.azure.com/acme` or `https://acme.visualstudio.com`.
2. **Project**: the project name.
3. **Read token (PAT)**: a personal access token with the **Work Items: Read** scope and nothing else, so
   it could not write even if there were a bug.
4. **Test connection and fetch states**: reads the project's process states.

The connection is per work folder; several folders can point to the same project, and they share its
state mapping and field choices.

Calls made with the read token: `GET` of `_apis/wit/workitemtypes`, `_apis/wit/fields` and
`_apis/wit/workitems` (in batches of 200), and `POST _apis/wit/wiql` for **Assigned this period**. WIQL
queries have no `GET` form, but the POST only reads and works with the Work Items: Read scope.

### Write token

**Change state in Azure DevOps** asks for a second token, **Work Items: Read & Write**, stored separately
(keyed by organization). It is used for one call only: a `PATCH` of `System.State` on each work item you
checked. Either token can be removed from Settings at any time. **Disconnect this folder** removes the link
and the cache of the last sync but keeps the tokens and the mapping.

Credentials are stored under the service `workboard` with keys like `azure-devops:read:<org>` and
`azure-devops:write:<org>` (`WORKBOARD_SECRET_SERVICE` changes the service name, which tests use to avoid
touching real credentials).

## State mapping

Each tracker state is mapped to a local state of your [workflow](workflow.md). Workboard suggests a mapping
from the tracker's category of each state, using the first local state of the matching category:

| Azure DevOps category | Workflow category |
|---|---|
| Proposed | `backlog` |
| In Progress | `active` |
| Resolved | `review` |
| Completed | `done` |
| Removed | `cancelled` |

Review the suggestions and **Save mapping**. Tracker states left unmapped are never proposed by the sync;
the sync preview lists them so you can map them on the spot. **Fetch again** picks up new states.

Types are matched by id, label or alias of the workflow's `types` (`User Story` → `feature` in the default
preset).

### Planning fields

**Due date** and **Agreed hours** come from fields you choose per project among the project's date and
numeric fields. If you don't choose:

- the due date is guessed from field names (`target date`, `due date`…);
- the hours field is the numeric field that sounds like hours/estimate/effort and has values in the most
  work items, falling back to Original Estimate.

Priority comes from `Microsoft.VSTS.Common.Priority` when the project has it. A README's own `due`,
`period` and `hours` always win over the tracker's.

## Sync

**Sync** reads every work item referenced by the folder's items (closed ones too) and stores the result in
`<config>/tracker_cache.json`. Then a preview proposes, per folder, the state the tracker implies through
the mapping, and/or the type the item is missing. Each row has a kind:

| Kind | Meaning | Checked by default |
|---|---|---|
| fills | The item has no status. | yes |
| type | The item has no type; the tracker's type maps to one. | yes |
| advances | The item has another status and the tracker's is further along. Shown when you tick *Include the ones that already have another local status*. | yes |
| moves back | The tracker's state ranks lower than the local one (for example, local *Testing* and the tracker says *Resolved*). | **no** |
| per git | The item's work is already in an environment in **all** its repos, and `git.envStates` maps that environment to a state further along than both the current and the proposed one. | **no** |

Ranking uses the workflow: category first (backlog < planned < active < review < done), then the state's
position in the preset. Cancelled and unknown states are off the scale and never count as moving back. When
a folder has several work items, the least advanced one decides.

**Apply** writes only the `status` and `type` fields of the checked folders (through the same safe README
writer as everything else). Nothing is written to the tracker. The preview also lists folders whose work
item was not found and how many already match.

## Assigned this period

**Assigned this period** lists your work items (assigned to the owner of the read token) whose due date
falls in a period, excluding closed states. Periods follow the workflow's `period.startDay`. The ones you
pick get a folder in the workflow's initial `backlog` state, named with `folders.newItemTemplate`, with the
title, type, description and acceptance criteria from the tracker. Work items that already have a folder
are marked and skipped. A due date field must be chosen first.

## New item from a work item

In **New item**, typing a work item id fetches its title, type, state, description and acceptance criteria
(HTML converted to Markdown). The README is created with:

```markdown
---
status: backlog
id: 4711
type: bug
---
# Login fails with SSO

> Bug #4711 · Azure DevOps: Active

## Description

…

## Acceptance criteria

…
```

The section titles come from the workflow's `sections`. Without a connection or token the form is filled
by hand.

## Change state in Azure DevOps

The reverse direction: for the items you select, Workboard proposes the tracker state the mapping links to
each local status (you can pick another), shows the current one, and after a confirmation changes them
with the write token. The cache is updated with what the tracker returned.

## What the MCP server reads

`tracker_cache.json` holds, per work folder, the last read of each work item: state, title, type, assignee,
changed date, priority, due date, estimate and completed work. The `workboard` MCP server reads it to
show priority, agreed date, period and agreed hours in `workboard_get_item` (marked as coming from the
tracker when the README has none). The MCP server never talks to the tracker and never sees a token.

## Adding a tracker

Trackers live in `crates/wb-tracker`. A new one needs:

1. **A client implementing `Tracker`** (`crates/wb-tracker/src/lib.rs`):

   ```rust
   pub trait Tracker: Send + Sync {
       fn states(&self) -> Result<Vec<TrackerState>>;      // with `suggested: Option<StateCategory>`
       fn fields(&self) -> Result<Vec<FieldInfo>>;
       fn work_items(&self, ids: &[String], req: &FieldRequest) -> Result<BTreeMap<String, WorkItem>>;
       fn assigned_to_me(&self, due_field: &str, start: &str, end: &str, exclude: &[String])
           -> Result<Vec<String>>;
       fn detail(&self, id: &str) -> Result<Detail>;
       fn set_state(&self, id: &str, state: &str) -> Result<StateChange>; // the only write
   }
   ```

   Everything except `set_state` must be read-only. Errors are `TrackerError { code, params }` with codes
   from `locales/*.json`; never put the token in them.

2. **HTTP through `Transport`.** The client receives an `Arc<dyn Transport>` and builds `HttpRequest`s; the
   app passes `UreqTransport` (ureq + rustls with the system's certificates, so it works behind TLS-
   inspecting proxies) and tests pass a fake that answers from fixed JSON. `wb-devserver --fake-tracker
   <file.json>` does the same for UI work: keys are `"<METHOD> <end of path>"`.

3. **Registration**: add its `KIND` to `KINDS` and a branch to `Connection::normalized`, `source`,
   `item_url` and `client`; add `tracker.kind.<kind>` to both locale files.

The rest (mapping, preview, cache, credentials, UI) is shared. Note that importing and fetching a work item
currently expect numeric ids, as Azure DevOps uses.
