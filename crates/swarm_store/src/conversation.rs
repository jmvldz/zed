use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub id: Uuid,
    pub role: MessageRole,
    pub content: String,
    pub timestamp: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Conversation {
    pub id: Uuid,
    pub title: Option<String>,
    pub codex_session_id: Option<String>,
    pub messages: Vec<Message>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Conversation {
    pub fn new() -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        Self {
            id: Uuid::new_v4(),
            title: None,
            codex_session_id: None,
            messages: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    pub fn with_id(id: Uuid) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        Self {
            id,
            title: None,
            codex_session_id: None,
            messages: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    pub fn add_message(&mut self, role: MessageRole, content: String) -> &Message {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let message = Message {
            id: Uuid::new_v4(),
            role,
            content,
            timestamp: now,
        };

        self.messages.push(message);
        self.updated_at = now;
        self.messages.last().expect("Just pushed a message")
    }

    pub fn set_title(&mut self, title: String) {
        self.title = Some(title);
        self.touch();
    }

    pub fn set_codex_session_id(&mut self, session_id: String) {
        self.codex_session_id = Some(session_id);
        self.touch();
    }

    fn touch(&mut self) {
        self.updated_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
    }

    pub fn generate_title(&self) -> String {
        if let Some(first_user_message) = self.messages.iter().find(|m| m.role == MessageRole::User) {
            let content = &first_user_message.content;
            if content.len() > 50 {
                format!("{}...", &content[..47])
            } else {
                content.clone()
            }
        } else {
            "New Conversation".to_string()
        }
    }
}

impl Default for Conversation {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ConversationStore {
    conversations: Vec<Conversation>,
    file_path: PathBuf,
}

impl ConversationStore {
    pub fn new() -> Self {
        Self {
            conversations: Vec::new(),
            file_path: Self::default_file_path(),
        }
    }

    fn default_file_path() -> PathBuf {
        paths::data_dir().join("swarm").join("conversations.json")
    }

    pub fn load() -> Result<Self> {
        let file_path = Self::default_file_path();

        if !file_path.exists() {
            log::info!("No conversations file found, starting fresh");
            return Ok(Self::new());
        }

        let json = std::fs::read_to_string(&file_path)
            .with_context(|| format!("Failed to read conversations from {:?}", file_path))?;

        let conversations: Vec<Conversation> = serde_json::from_str(&json)
            .with_context(|| "Failed to parse conversations JSON")?;

        log::info!("Loaded {} conversations from {:?}", conversations.len(), file_path);

        Ok(Self {
            conversations,
            file_path,
        })
    }

    pub fn save(&self) -> Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = self.file_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory {:?}", parent))?;
        }

        let json = serde_json::to_string_pretty(&self.conversations)
            .with_context(|| "Failed to serialize conversations")?;

        std::fs::write(&self.file_path, json)
            .with_context(|| format!("Failed to write conversations to {:?}", self.file_path))?;

        log::debug!("Saved {} conversations to {:?}", self.conversations.len(), self.file_path);

        Ok(())
    }

    pub fn add(&mut self, conversation: Conversation) {
        self.conversations.push(conversation);
    }

    pub fn get(&self, id: &Uuid) -> Option<&Conversation> {
        self.conversations.iter().find(|c| &c.id == id)
    }

    pub fn get_mut(&mut self, id: &Uuid) -> Option<&mut Conversation> {
        self.conversations.iter_mut().find(|c| &c.id == id)
    }

    pub fn remove(&mut self, id: &Uuid) -> Option<Conversation> {
        if let Some(pos) = self.conversations.iter().position(|c| &c.id == id) {
            Some(self.conversations.remove(pos))
        } else {
            None
        }
    }

    pub fn list(&self) -> &[Conversation] {
        &self.conversations
    }

    pub fn list_recent(&self, limit: usize) -> Vec<&Conversation> {
        let mut sorted: Vec<_> = self.conversations.iter().collect();
        sorted.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        sorted.into_iter().take(limit).collect()
    }

    pub fn conversations(&self) -> &[Conversation] {
        &self.conversations
    }

    pub fn is_empty(&self) -> bool {
        self.conversations.is_empty()
    }
}

impl Default for ConversationStore {
    fn default() -> Self {
        Self::new()
    }
}
