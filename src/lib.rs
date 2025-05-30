/// まなみのメインモジュール
/// メッセージ・コマンドのハンドリングを担当
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

use rand::prelude::IndexedRandom;
use rand::rng;

use dashmap::DashMap;

use regex::Regex;
use serenity::all::{MessageId, MessageUpdateEvent};
use serenity::{
    all::{ActivityData, Command},
    async_trait,
    builder::{CreateInteractionResponse, CreateInteractionResponseMessage},
    model::{
        application::Interaction,
        channel::Message,
        gateway::Ready,
        id::{ChannelId, GuildId, RoleId, UserId},
    },
    prelude::*,
};
use tracing::{error, info};

use calculator::EvalContext;
use commands::*;
use db::BotDatabase;

pub mod commands;

pub mod ai;
pub mod calculator;
pub mod cclemon;
pub mod db;
pub mod parser;

pub struct Bot {
    // Discordサーバーの情報
    // サーバーID
    pub guild_id: GuildId,

    // チャンネルのID
    pub channel_ids: Vec<ChannelId>,
    pub debug_channel_id: ChannelId,

    // まなみの情報
    // まなみのバージョン情報
    pub commit_hash: Option<String>,
    pub commit_date: Option<String>,

    // まなみの雑談用のAI
    pub gemini: ai::GeminiAI,

    // コマンド用のデータ
    // ログなどを保存するDB
    pub database: BotDatabase,

    // 全レスモードのデータ
    pub reply_to_all_mode: Arc<Mutex<ReplyToAllModeData>>,

    // 有効なコマンドのデータ
    pub slash_commands: Vec<ManamiSlashCommand>,
    pub prefix_commands: Vec<ManamiPrefixCommand>,

    // jailコマンドのデータ
    pub jail_process: Arc<DashMap<UserId, (usize, Instant)>>,
    pub jail_id: Arc<Mutex<usize>>,
    // Jail用のRoleのID
    pub jail_mark_role_id: RoleId,
    pub jail_main_role_id: RoleId,

    // var, calcコマンドのデータ
    pub variables: EvalContext,
}

impl Bot {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        channel_ids: Vec<ChannelId>,
        debug_channel_id: ChannelId,

        guild_id: GuildId,
        jail_mark_role_id: RoleId,
        jail_main_role_id: RoleId,

        gemini: ai::GeminiAI,

        commit_hash: Option<String>,
        commit_date: Option<String>,

        disabled_commands: &[&str],

        database: BotDatabase,
    ) -> Self {
        let variables = database.retrieve_eval_context().await;
        let jail_process = Arc::new(DashMap::new());
        let jail_id = Arc::new(Mutex::new(0));
        let reply_to_all_mode: Arc<Mutex<ReplyToAllModeData>> =
            Arc::new(Mutex::new(ReplyToAllModeData::blank()));
        let prefix_commands = prefix_commands(disabled_commands);
        let slash_commands = slash_commands(disabled_commands);

        Self {
            jail_process,
            jail_id,
            channel_ids,
            debug_channel_id,
            guild_id,
            jail_mark_role_id,
            jail_main_role_id,
            commit_hash,
            commit_date,
            variables,
            reply_to_all_mode,
            gemini,
            prefix_commands,
            slash_commands,
            database,
        }
    }

    pub async fn get_user_room_pointer(&self, user_id: &UserId) -> ChannelId {
        let default_channel_id = self.channel_ids[0];
        self.database
            .fetch_user_room_pointer(user_id, default_channel_id)
            .await
            .unwrap_or(default_channel_id)
    }

    pub async fn change_room_pointer(
        &self,
        userid: &UserId,
        username: &str,
        room_pointer: ChannelId,
    ) -> Result<(), anyhow::Error> {
        self.database
            .set_user_room_pointer(userid, username, Some(room_pointer))
            .await
            .unwrap();
        Ok(())
    }
}

#[async_trait]
impl EventHandler for Bot {
    async fn ready(&self, ctx: Context, ready: Ready) {
        info!("{} is connected!", ready.user.name);

        if let (Some(commit_hash), Some(commit_date)) =
            (self.commit_hash.clone(), self.commit_date.clone())
        {
            ctx.set_activity(Some(ActivityData::custom(format!(
                "commit: {commit_hash} ({commit_date})"
            ))));
        };

        let message_hello_list = [
            "おはようっ！",
            "やっほー！",
            "ふわぁ、今日もがんばるよー！",
            "あれっ？　寝ちゃってた？",
            "おすし買ってきたよー！",
            "たーだいまーっ！",
            "まなみちゃん、参上！",
            "おはよう、みんな！",
            "だれかいるー？",
            "まなみは元気ですっ！",
        ];

        let message_hello = *message_hello_list.choose(&mut rng()).unwrap();

        if let Err(why) = self.debug_channel_id.say(&ctx.http, message_hello).await {
            error!("Error sending message: {:?}", why);
        };

        // ローカルコマンドの登録
        let _ = self
            .guild_id
            .set_commands(
                &ctx.http,
                self.slash_commands
                    .iter()
                    .filter(|cmd| cmd.is_local_command)
                    .map(|cmd| (cmd.register)())
                    .collect::<Vec<_>>(),
            )
            .await;

        // グローバルコマンドの登録
        for command in self
            .slash_commands
            .iter()
            .filter(|cmd| !cmd.is_local_command)
        {
            let _ = Command::create_global_command(&ctx.http, (command.register)()).await;
        }

        // roles のいずれかが付いているユーザーを恩赦
        let guild = self.guild_id;
        let roles = [self.jail_mark_role_id, self.jail_main_role_id];
        let members = guild.members(&ctx.http, None, None).await.unwrap();

        for member in members {
            if member.roles.iter().any(|role| roles.contains(role)) {
                let command_context = commands::CommandContext {
                    bot: self,
                    ctx: &ctx,
                    channel_id: self.debug_channel_id,
                    author_id: member.user.id,
                    guild_id: Some(guild),
                    command: "".to_owned(),
                };
                unjail::run(command_context).await;
            }
        }
    }

    async fn message(&self, ctx: Context, msg: Message) {
        // Check if the message is from direct message
        match msg.guild_id {
            Some(_) => {
                guild_message(self, &ctx, &msg).await;
            }
            None => {
                direct_message(self, &ctx, &msg).await;
            }
        }
    }

    async fn message_update(
        &self,
        _: Context,
        _: Option<Message>,
        new: Option<Message>,
        _: MessageUpdateEvent,
    ) {
        if let Some(new) = new {
            if let Err(e) = self.database.update_guild_message(&new).await {
                error!("Error updating message: {e:?}");
            }
        }
    }

    async fn message_delete(
        &self,
        _: Context,
        _: ChannelId,
        deleted_message_id: MessageId,
        _: Option<GuildId>,
    ) {
        if let Err(e) = self
            .database
            .delete_guild_message(&deleted_message_id)
            .await
        {
            error!("Error deleting message: {e:?}");
        }
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        if let Interaction::Command(command) = interaction {
            #[allow(unused_variables)]
            let command_context = CommandContext::new_from_command_interaction(
                self,
                &ctx,
                &command,
                &command.data.name,
            );
            let content = match self
                .slash_commands
                .iter()
                .find(|cmd| cmd.name == command.data.name)
            {
                Some(cmd) => (cmd.run)(command.data.options(), command_context).await,
                None => "知らないコマンドだよ！".to_owned(),
            };

            let data = CreateInteractionResponseMessage::new().content(content);
            let builder = CreateInteractionResponse::Message(data);
            if let Err(why) = command.create_response(&ctx.http, builder).await {
                error!("Error sending message: {:?}", why);
            }
        }
    }
}

//direct message
async fn direct_message(bot: &Bot, ctx: &Context, msg: &Message) {
    if !has_privilege(bot, ctx, msg).await {
        return;
    }

    // if message is not command, forward to the room
    if !msg.content.starts_with('!') {
        // 代筆先のチャンネルにメッセージを転送する
        let room_pointer = bot.get_user_room_pointer(&msg.author.id).await;
        if let Err(why) = room_pointer.say(&ctx.http, &msg.content).await {
            error!("Error sending message: {:?}", why);
        };
        return;
    }

    let command_context = CommandContext::new(bot, ctx, msg, msg.content[1..].to_string());

    // if message is command, handle command
    // handle command
    let command_name = &command_context.command_name()[..];

    match bot
        .prefix_commands
        .iter()
        .filter(|cmd| cmd.is_dm_command)
        .find(|cmd| cmd.name == command_name || cmd.alias.contains(&command_name))
    {
        Some(cmd) => (cmd.run)(command_context).await,
        None => {
            let _ = &msg
                .channel_id
                .say(&ctx.http, "しらないコマンドだよ")
                .await
                .unwrap();
        }
    }
}

async fn save_guild_message(bot: &Bot, ctx: &Context, msg: &Message) {
    let user_name = msg.author_nick(&ctx.http).await;
    let user_name = user_name
        .as_deref()
        .unwrap_or_else(|| msg.author.display_name());
    let channel_name = msg.channel_id.name(&ctx.http).await;
    let channel_name = channel_name.as_deref().unwrap_or("");

    if let Err(e) = bot
        .database
        .insert_guild_message(msg, user_name, channel_name)
        .await
    {
        error!("Error adding message: {e:?}");
    }

    // AIのためにメッセージを保存する
    if msg.channel_id.get() == bot.debug_channel_id.get() && !msg.author.bot {
        bot.gemini.add_user_log(user_name, &msg.content);
    }
}

async fn guild_message(bot: &Bot, ctx: &Context, msg: &Message) {
    save_guild_message(bot, ctx, msg).await;

    // 反応すべきメッセージかどうか確認
    if !has_privilege(bot, ctx, msg).await {
        return;
    }

    // 全レスモード中？
    let response_to_all = bot.reply_to_all_mode.lock().unwrap().is_active();
    let response_to_all_model = bot.reply_to_all_mode.lock().unwrap().model.clone();

    // if message does not contains any command, ignore
    let command_pattern =
        Regex::new(r"(?ms)((?:まなみ(?:ちゃん)?(?:\s|、|は|って|の)?)|!)(.*)").unwrap();
    let (_prefix, input_string): (String, String) = match command_pattern.captures(&msg.content) {
        Some(caps) => (
            caps.get(1).unwrap().as_str().to_owned(),
            caps.get(2).unwrap().as_str().to_owned(),
        ),
        None => {
            // 全レスモードの場合のみ返答
            if msg.channel_id.get() == bot.debug_channel_id.get() && response_to_all {
                bot.reply_to_all_mode.lock().unwrap().renew(); // 期限更新
                let content = bot.gemini.generate_with_model(response_to_all_model).await;
                let content = match content {
                    Ok(content) => content.replace("うだまなみ: ", ""),
                    Err(e) => {
                        format!("Error sending message: {e:?}")
                    }
                };
                let _ = &msg.channel_id.say(&ctx.http, content).await;
            }
            return;
        }
    };

    let command_context = CommandContext::new(bot, ctx, msg, input_string.clone());

    // handle other command
    let command_name = &command_context.command_name()[..];

    match bot
        .prefix_commands
        .iter()
        .filter(|cmd| cmd.is_guild_command)
        .find(|cmd| cmd.name == command_name || cmd.alias.contains(&command_name))
    {
        Some(cmd) => (cmd.run)(command_context).await,
        None => {
            if msg.content.starts_with("!") {
                if let Some(cmd) = bot
                    .prefix_commands
                    .iter()
                    .filter(|cmd| cmd.is_guild_command)
                    .find(|cmd| cmd.name == "dice")
                {
                    return (cmd.run)(command_context).await;
                }
            }

            if msg.channel_id.get() == bot.debug_channel_id.get() {
                // まなみが自由に応答するコーナー
                let content = if response_to_all {
                    bot.reply_to_all_mode.lock().unwrap().renew(); // 期限更新
                                                                   // ↓全レスモードなら全レス用のモデルを使用
                    bot.gemini.generate_with_model(response_to_all_model).await
                } else {
                    bot.gemini.generate().await
                };
                let content = match content {
                    Ok(content) => content.replace("うだまなみ: ", ""),
                    Err(e) => {
                        format!("Error sending message: {e:?}")
                    }
                };
                let _ = &msg.channel_id.say(&ctx.http, content).await;
            }
        }
    }
}

async fn has_privilege(bot: &Bot, ctx: &Context, msg: &Message) -> bool {
    if msg.author.bot {
        return false;
    }
    // ユーザーがこっちにきてはいけないに存在しない場合は無視
    if bot
        .guild_id
        .member(&ctx.http, &msg.author.id)
        .await
        .is_err()
    {
        return false;
    }
    true
}
