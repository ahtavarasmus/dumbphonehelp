use serde::{Serialize, Deserialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};
use super::schema::reminders;
use diesel::{Queryable, Insertable};


#[derive(Queryable, Insertable, Serialize, Deserialize, Debug, Clone)]
#[diesel(table_name = reminders)]
pub struct Reminder {
    pub id: String,
    pub message: String,
    pub remind_at: String,
}

#[derive(Deserialize)]
pub struct CreateReminder {
    pub message: String,
    pub remind_at: String,
}

impl Reminder {
    pub fn new(message: String, remind_at: String) -> Self {
        Reminder {
            id: Uuid::new_v4().to_string(),
            message,
            remind_at,
        }
    }
    pub fn into_datetime(&self) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(&self.remind_at)
            .unwrap()
            .with_timezone(&Utc)
    }
}

#[derive(Serialize, Debug)]
#[serde(untagged)]
pub enum ToolCallResponse {
    Single(Reminder),
    Multiple(Vec<Reminder>),
    Message(String),
}

#[derive(Serialize, Debug)]
pub struct ToolCallResult {
    pub toolCallId: String,
    pub result: ToolCallResponse,
}

#[derive(Serialize, Debug)]
pub struct ResponseWrapper {
    pub results: Vec<ToolCallResult>,
}
