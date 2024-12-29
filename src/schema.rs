// @generated automatically by Diesel CLI.

diesel::table! {
    reminders (id) {
        id -> Text,
        message -> Text,
        remind_at -> Text,
    }
}
