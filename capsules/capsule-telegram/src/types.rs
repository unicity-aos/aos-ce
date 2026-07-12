//! Telegram Bot API types (minimal subset).

use serde::{Deserialize, Serialize};

/// Generic Telegram API response wrapper.
#[derive(Debug, Deserialize)]
pub struct TgResponse<T> {
    pub ok: bool,
    pub result: Option<T>,
    pub description: Option<String>,
}

/// A Telegram update from `getUpdates`.
#[derive(Debug, Deserialize)]
pub struct Update {
    pub update_id: i64,
    pub message: Option<TgMessage>,
    pub callback_query: Option<CallbackQuery>,
}

/// A Telegram message.
#[derive(Debug, Deserialize)]
pub struct TgMessage {
    pub message_id: i64,
    pub from: Option<TgUser>,
    pub chat: TgChat,
    pub text: Option<String>,
}

/// A Telegram user.
#[derive(Debug, Deserialize)]
pub struct TgUser {
    pub id: i64,
    #[allow(dead_code)]
    pub first_name: String,
}

/// A Telegram chat.
#[derive(Debug, Deserialize)]
pub struct TgChat {
    pub id: i64,
}

/// A callback query from an inline keyboard button press.
#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    pub id: String,
    pub from: TgUser,
    pub message: Option<TgMessage>,
    pub data: Option<String>,
}

/// Inline keyboard markup for approval/elicitation buttons.
#[derive(Debug, Serialize)]
pub struct InlineKeyboardMarkup {
    pub inline_keyboard: Vec<Vec<InlineKeyboardButton>>,
}

/// A single inline keyboard button.
#[derive(Debug, Serialize)]
pub struct InlineKeyboardButton {
    pub text: String,
    pub callback_data: String,
}
