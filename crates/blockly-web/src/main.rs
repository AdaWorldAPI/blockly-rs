//! The showcase server — one page, one endpoint, one binary.
//!
//! Real Blockly (loaded from a pinned CDN build, per the standing rule:
//! *"JavaScript keeps dragging puzzle pieces; Rust owns semantics"* — a block
//! renderer is never ported) drags blocks in the browser; every change POSTs
//! the workspace save to `/api/cast`, which runs the shipped pipeline —
//! `blockly-shim` → `lower_program` → stored-node bytes — and returns what
//! the panels show, refusals included.
//!
//! Binds `0.0.0.0:$PORT` (Railway injects `PORT`; 8080 is only the local
//! fallback — never hardcoded as the deploy port).

mod cast;
mod stage;
mod surface;

use askama::Template;
use axum::extract::Query;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate;

#[derive(Deserialize)]
struct ToolboxQuery {
    /// `blockly` (default) or `scratch`. Anything else falls back to
    /// `blockly`, the same forgiving-field rule the shape selector uses.
    dialect: Option<String>,
}

#[derive(Deserialize)]
struct TemplateQuery {
    /// Which built-in reference program to load.
    name: Option<String>,
}

#[derive(Deserialize)]
struct RunQuery {
    /// Lane shape, as elsewhere.
    shape: Option<String>,
    /// How many calls the run may execute before it stops. Bounds `forever`.
    budget: Option<u32>,
    /// Mouse position the sensing reporters see.
    mouse_y: Option<f32>,
    /// Whether `sensing_touchingobject` answers true.
    touching: Option<bool>,
}

#[derive(Deserialize)]
struct CastQuery {
    /// `pairs` (default) / `triples` / `quads` — anything else falls back to
    /// `pairs`, the same forgiving-field rule the workspace's other demo
    /// selectors use.
    shape: Option<String>,
}

async fn index() -> Html<String> {
    Html(IndexTemplate.render().expect("static template renders"))
}

async fn api_cast(Query(q): Query<CastQuery>, body: String) -> Json<cast::CastOut> {
    Json(cast::cast_workspace(
        &body,
        q.shape.as_deref().unwrap_or("pairs"),
    ))
}

/// The toolbox, built from `blockly_abi::codebook::CATEGORIES`.
///
/// Served rather than hand-written in the page, because a hand-written
/// toolbox is a second vocabulary beside the codebook and drifts silently in
/// the direction that hurts — this demo shipped offering 18 block types while
/// the cast already handled 64, so every list, variable and procedure block
/// was invisible. Now the page cannot offer a block the cast refuses, and
/// cannot hide one it handles: the codebook's own completeness test
/// (`every_resolvable_type_is_listed_and_every_listed_type_resolves`) pins
/// both directions.
async fn api_toolbox(Query(q): Query<ToolboxQuery>) -> Json<serde_json::Value> {
    // Scratch tiles carry the family key as their colour hook; Blockly tiles
    // use Blockly's own built-in category styles. Either way the TYPES come
    // from the codebook, never from a list typed into the page.
    let contents: Vec<_> = if q.dialect.as_deref() == Some("scratch") {
        blockly_abi::scratch::SCRATCH_CATEGORIES
            .iter()
            .map(|(label, family, types)| {
                serde_json::json!({
                    "kind": "category",
                    "name": label,
                    "colour": scratch_colour(family),
                    "contents": blocks(types),
                })
            })
            .collect()
    } else {
        blockly_abi::codebook::CATEGORIES
            .iter()
            .map(|(name, types)| {
                serde_json::json!({
                    "kind": "category",
                    "name": name,
                    "categorystyle": category_style(name),
                    "contents": blocks(types),
                })
            })
            .collect()
    };
    Json(serde_json::json!({"kind": "categoryToolbox", "contents": contents}))
}

/// Blockly JSON block definitions for the Scratch tiles.
///
/// Vanilla Blockly ships Blockly's blocks, not Scratch's, so the page has no
/// definition for `motion_movesteps` and a Scratch toolbox would reference
/// undefined types. These are generated from
/// [`blockly_abi::scratch::SCRATCH_BLOCK_DEFS`] — the same harvested rows
/// that mint the opcodes — so a tile cannot exist without the operation
/// behind it, and its input count cannot disagree with the arity the palette
/// reports. The label is the opcode itself: honest, and it makes the
/// address visible while dragging.
async fn api_scratch_defs() -> Json<serde_json::Value> {
    use blockly_abi::scratch::Shape;
    let defs: Vec<_> = blockly_abi::scratch::SCRATCH_BLOCK_DEFS
        .iter()
        .map(|&(ty, family, shape, values, stmts)| {
            // Sockets carry SCRATCH's own names (CONDITION / SUBSTACK / STEPS),
            // harvested with the opcodes. That is what lets a stored template
            // and these generated definitions agree without a translation
            // table between them.
            let mut message = ty.to_string();
            let mut args = Vec::new();
            for (i, name) in values.iter().enumerate() {
                message.push_str(&format!(" %{}", i + 1));
                args.push(serde_json::json!({"type": "input_value", "name": name}));
            }
            for (i, name) in stmts.iter().enumerate() {
                message.push_str(&format!(" %{}", values.len() + i + 1));
                args.push(serde_json::json!({"type": "input_statement", "name": name}));
            }
            let mut d = serde_json::json!({
                "type": ty,
                "message0": message,
                "args0": args,
                "colour": scratch_colour(family),
                "tooltip": format!("{ty} — {}", match shape {
                    Shape::Hat => "event hat",
                    Shape::Statement => "statement",
                    Shape::Reporter => "reporter",
                    Shape::Boolean => "boolean",
                }),
            });
            let o = d.as_object_mut().expect("just built as an object");
            match shape {
                // A hat starts a stack: nothing connects above it.
                Shape::Hat => {
                    o.insert("nextStatement".into(), serde_json::Value::Null);
                }
                Shape::Statement => {
                    o.insert("previousStatement".into(), serde_json::Value::Null);
                    o.insert("nextStatement".into(), serde_json::Value::Null);
                }
                Shape::Reporter => {
                    o.insert("output".into(), serde_json::Value::Null);
                }
                Shape::Boolean => {
                    o.insert("output".into(), serde_json::json!("Boolean"));
                }
            }
            d
        })
        .collect();
    Json(serde_json::Value::Array(defs))
}

/// A built-in reference program — RAISED FROM ITS STORED NODES.
///
/// The template ships as bytes (`templates/pong.nodes`, 8 nodes x 512 B), and
/// this endpoint reconstructs the blocks from them and renders the editor's
/// JSON on demand. That inverts what the demo used to do: the program is the
/// artefact, and Blockly's save format is a projection produced at the
/// membrane — the same posture as `/api/surface`, which renders HTML from the
/// same nodes.
///
/// The JSON copy still exists as the AUTHORING form so a template stays
/// reviewable, and a test asserts the two describe the same program, so the
/// bytes can never silently drift from the form a human reads.
async fn api_template(Query(q): Query<TemplateQuery>) -> Result<String, StatusCode> {
    let want = q.name.as_deref().unwrap_or("pong");
    let (_, nodes) = blockly_shim::templates::ALL_NODES
        .iter()
        .find(|(n, _)| *n == want)
        .ok_or(StatusCode::NOT_FOUND)?;
    let scripts = blockly_shim::templates::raise_nodes(nodes)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(blockly_shim::emit::to_workspace_json(&scripts))
}

/// The cast program as **bytes** — the canonical surface, no serialization.
///
/// `Frame::NodeDelta(...).to_le_bytes()`: the 512-byte stored node travels as
/// itself, addressed by its 16-byte key, with a changed-field mask the client
/// resolves through the classid's ClassView. This is what `/api/cast`'s JSON
/// should have been from the start — the arc's whole claim is that the node IS
/// the program, and a JSON description of it says the opposite.
async fn api_frame(Query(q): Query<CastQuery>, body: String) -> Response {
    let shape = q.shape.as_deref().unwrap_or("pairs");
    match cast::first_program(&body, shape) {
        Some(prog) => {
            let bytes = surface::program_frame_bytes(cast::demo_key(), &prog);
            ([(header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response()
        }
        // No castable script is not an error — an empty canvas is legal. An
        // empty body is the honest answer: zero nodes changed.
        None => (
            [(header::CONTENT_TYPE, "application/octet-stream")],
            Vec::new(),
        )
            .into_response(),
    }
}

/// The surface, rendered SERVER-SIDE through the upstream askama brick.
///
/// The page receives HTML that a ClassView projection produced, not a document
/// it has to lay out itself. That is the a2ui posture: the render happens from
/// the projection, and the client holds the codebook rather than the schema.
async fn api_surface(Query(q): Query<CastQuery>, body: String) -> Result<Html<String>, StatusCode> {
    let shape = q.shape.as_deref().unwrap_or("pairs");
    let Some(prog) = cast::first_program(&body, shape) else {
        return Ok(Html(String::new()));
    };
    surface::render_surface(cast::demo_key(), &prog, shape)
        .map(Html)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// RUN the cast program and render the resulting stage.
///
/// The interpreter reads `FunctionBody` call rails — the stored bytes — so
/// what this draws is the result of EXECUTING the program, not an animation
/// of the block layout. That is the last step of the arc's claim: the node is
/// not merely the storage form, it is the thing that runs.
///
/// `budget` bounds the run so a `forever` terminates. It is a property of the
/// run, never of the program.
async fn api_run(Query(q): Query<RunQuery>, body: String) -> Html<String> {
    let shape = q.shape.as_deref().unwrap_or("pairs");
    let budget = q.budget.unwrap_or(600).min(200_000);
    let Some(prog) = cast::first_program(&body, shape) else {
        return Html(stage::svg(&blockly_run::Stage::default(), false));
    };
    let mut m = blockly_run::Machine::new(&prog.functions, budget);
    m.stage.mouse_y = q.mouse_y.unwrap_or(0.0);
    m.stage.touching = q.touching.unwrap_or(false);
    // A refusal is INFORMATION, not a failure to hide: the stage still draws,
    // and the note says which operation stopped the run.
    let note = match m.run() {
        Ok(()) => None,
        Err(e) => Some(e.to_string()),
    };
    let mut svg = stage::svg(&m.stage, true);
    if let Some(n) = note {
        svg.push_str(&format!(
            "<p class=\"err\" style=\"margin:.3rem 0 0\">run stopped: {n}</p>"
        ));
    }
    Html(svg)
}

fn blocks(types: &[&str]) -> Vec<serde_json::Value> {
    types
        .iter()
        .map(|t| serde_json::json!({"kind": "block", "type": t}))
        .collect()
}

/// Tile colour per Scratch family.
///
/// **Presentation, not harvested data.** Scratch's palette colours are no
/// longer defined in `scratch-blocks` — the block definitions reference
/// `colours_<family>` extensions that the GUI registers — so these are a
/// deliberate local choice, chosen to read as the familiar Scratch palette
/// without being claimed as sourced. The opcode tables, unlike this, ARE
/// byte-exact from the Apache-2.0 source.
fn scratch_colour(family: &str) -> &'static str {
    match family {
        "motion" => "#4C97FF",
        "looks" => "#9966FF",
        "sound" => "#CF63CF",
        "event" => "#FFBF00",
        "control" => "#FFAB19",
        "sensing" => "#5CB1D6",
        "operators" => "#59C059",
        "data" => "#FF8C1A",
        "procedures" => "#FF6680",
        _ => "#9E9E9E",
    }
}

/// Blockly's own built-in category styles, so the tiles carry the colours a
/// block editor user already knows. Presentation only — never routing.
fn category_style(name: &str) -> &'static str {
    match name {
        "Logic" => "logic_category",
        "Loops" => "loop_category",
        "Math" => "math_category",
        "Text" => "text_category",
        "Lists" => "list_category",
        "Variables" => "variable_category",
        "Procedures" => "procedure_category",
        _ => "colour_category",
    }
}

#[tokio::main]
async fn main() {
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    let app = Router::new()
        .route("/", get(index))
        .route("/api/toolbox", get(api_toolbox))
        .route("/api/scratch-defs", get(api_scratch_defs))
        .route("/api/template", get(api_template))
        .route("/api/frame", post(api_frame))
        .route("/api/surface", post(api_surface))
        .route("/api/run", post(api_run))
        .route("/api/cast", post(api_cast));
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| panic!("cannot bind {addr}: {e}"));
    println!("blockly-web listening on http://{addr}");
    axum::serve(listener, app).await.expect("server runs");
}
