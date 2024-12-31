use axum::{
    extract::{State, Request},
    routing::{post, get},
    response::{Json, Response},
    http::StatusCode,
    body::Body,
    middleware::{self, Next},
    Router,
};
use serde_json::json;
use axum::debug_handler;
use tracing_subscriber::field::debug;
use crate::lib::establish_connection;
use crate::models::{Reminder, CreateReminder, ResponseWrapper, ToolCallResult, ToolCallResponse};
use std::sync::{Arc, Mutex};
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use tracing::Level;
use tower_http::trace::{self, TraceLayer};

use http_body_util::BodyExt;


pub mod models;
pub mod lib;
pub mod schema;

async fn log_request(
    req: Request,
    next: Next,
) -> Response {
    let (parts, body) = req.into_parts();
    
    // Print headers
    tracing::info!("Request Headers: {:#?}", parts.headers);
    
    // Get and print body
    let bytes = body.collect().await
        .map(|collected| collected.to_bytes())
        .unwrap_or_default();
    
    tracing::info!("Request Body: {}", String::from_utf8_lossy(&bytes));
    
    // Reconstruct the request and continue
    let req = Request::from_parts(parts, Body::from(bytes));
    next.run(req).await
}



#[tokio::main]
async fn main() {
    // Initialize tracing
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "debug,tower_http=debug");
    }
    tracing_subscriber::fmt()
        .with_target(false)
        .compact()
        .init();
    // build our application with a single route
    let app_state= Arc::new(Mutex::new(establish_connection()));
    let app = Router::new()
        .route("/tool-call", post(handle_tool_call))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(trace::DefaultMakeSpan::new().level(Level::INFO))
                .on_response(trace::DefaultOnResponse::new().level(Level::INFO))
        )
        .layer(middleware::from_fn(log_request))
        .with_state(app_state);

    // run our app with hyper, listening globally on port 3000
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    tracing::info!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}


#[derive(Deserialize, Debug)]
struct ToolCallRequest {
    message: ToolCallMessage,
}

#[derive(Deserialize, Debug)]
struct ToolCallMessage {
    #[serde(rename = "toolCalls")]
    tool_calls: Vec<ToolCall>,
}

#[derive(Deserialize, Debug)]
struct ToolCall {
    id: String,
    function: FunctionCall,
}

#[derive(Deserialize, Debug)]
struct FunctionCall {
    name: String,
    arguments: FunctionArgs,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)] // This allows for multiple possible structures
enum FunctionArgs {
    Create(CreateReminderArgs),
    Message(PerplexityMessageArgs),
    Empty(EmptyArgs),
}

#[derive(Deserialize, Debug, Clone)]
struct EmptyArgs {}

#[derive(Deserialize, Debug, Clone)]
struct PerplexityMessageArgs {
    message: String,
}

#[derive(Deserialize, Debug, Clone)]
struct CreateReminderArgs {
    message: String,
    remind_at: String,
}


#[debug_handler]
async fn handle_tool_call(
    State(pool): State<Arc<Mutex<SqliteConnection>>>,
    Json(payload): Json<ToolCallRequest>,
) -> Result<Json<ResponseWrapper>, (StatusCode, String)> {
    tracing::info!("Handling tool call");

    let mut results = Vec::new();

    for tool_call in payload.message.tool_calls.iter() {
        tracing::info!("Handling tool call : {:#?}", tool_call);
        let result = match (&tool_call.function.name, &tool_call.function.arguments) {
            (name, FunctionArgs::Empty(_)) if name == "GetUserReminders" => {
                let mut conn = pool.lock().unwrap();
                tracing::info!("Listing all reminders");
                let reminders = list_reminders(&mut conn)
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                ToolCallResponse::Multiple(reminders)
            },
            (name, FunctionArgs::Create(args)) if name == "StoreUserReminder" => {
                let mut conn = pool.lock().unwrap();
                tracing::info!("Creating reminder with args: {:#?}", args);
                let reminder = create_reminder(&mut conn, args)
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                ToolCallResponse::Single(reminder)
            },
            (name, FunctionArgs::Empty(_)) if name == "DeleteAllReminders" => {
                let mut conn = pool.lock().unwrap();
                tracing::info!("Deleting all reminders");
                let deleted_count = delete_all_reminders(&mut conn)
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                ToolCallResponse::Multiple(Vec::new()) // Return empty vector after deletion
            },
            (name, FunctionArgs::Message(args)) if name == "AskPerplexity" => {
                tracing::info!("Asking Perplexity");
                let response = ask_perplexity(&args.message)
                    .await
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

                ToolCallResponse::Message(response)
            },
            _ => {
                tracing::warn!("Unknown function call: {:#?}", tool_call.function.name);
                ToolCallResponse::Message("Unknown function call".to_string())
            }

        };

        results.push(ToolCallResult {
            toolCallId: tool_call.id.clone(),
            result,
        });
    }

    tracing::info!("Returning results: {:#?}", results);
    Ok(Json(ResponseWrapper { results }))
}


async fn ask_perplexity(message: &str) -> Result<String, reqwest::Error> {
    let api_key = std::env::var("PERPLEXITY_API_KEY").expect("PERPLEXITY_API_KEY must be set");
    let client = reqwest::Client::new();
    
    let payload = json!({
        "model": "llama-3.1-sonar-small-128k-online",
        "messages": [
            {
                "role": "system",
                "content": "Be precise and concise."
            },
            {
                "role": "user",
                "content": message
            }
        ]
    });

    let response = client
        .post("https://api.perplexity.ai/chat/completions")
        .header("accept", "application/json")
        .header("content-type", "application/json")
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&payload)
        .send()
        .await?;

    let result = response.text().await?;
    println!("{}", result);
    Ok(result)
}

fn create_reminder(
    conn: &mut SqliteConnection,
    args: &CreateReminderArgs,
) -> Result<Reminder, diesel::result::Error> {
    tracing::info!("Creating a new reminder");
    tracing::info!("Message: {}", args.message);
    tracing::info!("Remind at: {}", args.remind_at);
    use crate::schema::reminders::dsl::*;


    let new_reminder = Reminder::new(args.message.clone(), args.remind_at.clone());
    
    let result = diesel::insert_into(reminders)
        .values(&new_reminder)
        .execute(conn);

    tracing::info!("result from the insert: {:#?}", result);

    tracing::info!("Created reminder: {:#?}", new_reminder);
    Ok(new_reminder)
}



fn list_reminders(
    conn: &mut SqliteConnection
) -> Result<Vec<Reminder>, diesel::result::Error> {
    tracing::debug!("Listing all reminders");
    use crate::schema::reminders::dsl::*;

    reminders.load::<Reminder>(&mut *conn)
}

fn delete_all_reminders(
    conn: &mut SqliteConnection
) -> Result<usize, diesel::result::Error> {
    use crate::schema::reminders::dsl::*;
    
    diesel::delete(reminders).execute(conn)
}
