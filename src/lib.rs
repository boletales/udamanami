use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use dashmap::DashMap;

use calculator::EvalResult;
use nom::{
    error::{Error, ErrorKind},
    Finish as _,
};
use regex::Regex;
use serenity::{
    async_trait,
    model::{
        channel::Message,
        gateway::Ready,
        id::{ChannelId, GuildId, RoleId, UserId},
    },
    prelude::*,
    utils::{parse_user_mention, MessageBuilder},
};
use tokio::{spawn, time::sleep};
use tracing::{error, info};

use calculator::val_as_str;
use commands::*;

pub mod commands;

pub mod ai;
pub mod calculator;
pub mod cclemon;
pub mod parser;

#[derive(Clone)]
pub struct UserData {
    pub room_pointer: ChannelId,
}

pub struct Bot {
    pub userdata: DashMap<UserId, UserData>,
    pub jail_process: Arc<DashMap<UserId, (usize, Instant)>>,
    pub jail_id: Arc<Mutex<usize>>,

    pub channel_ids: Vec<ChannelId>,
    pub guild_id: GuildId,
    pub erogaki_role_id: RoleId,
    pub jail_mark_role_id: RoleId,
    pub jail_main_role_id: RoleId,

    pub variables: DashMap<String, EvalResult>,
    pub ai: ai::AI,
    pub chat_log: DashMap<ChannelId, Mutex<VecDeque<(String, Message)>>>,
}

impl Bot {
    pub fn get_user_room_pointer(&self, user_id: &UserId) -> ChannelId {
        self.userdata
            .entry(*user_id)
            .or_insert(UserData {
                room_pointer: self.channel_ids[0],
            })
            .clone()
            .room_pointer
    }

    pub fn change_room_pointer(
        &self,
        userid: &UserId,
        room_pointer: ChannelId,
    ) -> Result<(), anyhow::Error> {
        self.userdata
            .entry(*userid)
            .or_insert(UserData {
                room_pointer: self.channel_ids[0],
            })
            .room_pointer = room_pointer;
        Ok(())
    }
}

#[async_trait]
impl EventHandler for Bot {
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

    async fn ready(&self, ctx: Context, ready: Ready) {
        info!("{} is connected!", ready.user.name);
        if let Err(why) = self.channel_ids[4].say(&ctx.http, "おはようっ！").await {
            error!("Error sending message: {:?}", why);
        };

        // roles のいずれかが付いているユーザーを恩赦
        let guild = self.guild_id;
        let roles = vec![self.jail_mark_role_id, self.jail_main_role_id];
        let members = guild.members(&ctx.http, None, None).await.unwrap();
        for member in members {
            if member.roles.iter().any(|role| roles.contains(role)) {
                unjail(
                    &self.channel_ids[4],
                    &ctx,
                    &member.user.id,
                    &guild,
                    &roles,
                    None,
                    &self.jail_process,
                )
                .await;
            }
        }
    }
}

//direct message
async fn direct_message(bot: &Bot, ctx: &Context, msg: &Message) {
    if !has_privilege(bot, ctx, msg).await {
        return;
    }

    let room_pointer = bot.get_user_room_pointer(&msg.author.id);

    // if message is not command, forward to the room
    if !msg.content.starts_with('!') {
        if let Err(why) = room_pointer.say(&ctx.http, &msg.content).await {
            error!("Error sending message: {:?}", why);
        };
        return;
    }

    // if message is command, handle command
    // handle command
    let split_message = msg.content.split_whitespace().collect::<Vec<&str>>();
    let command_name = &split_message[0][1..]; // 先頭の "!" を削除
    let command_args = &split_message[1..];
    let dm = &msg.channel_id;

    let command_context = CommandContext {
        bot,
        http: &ctx.http,
        msg,
        guild_id: Some(bot.guild_id),
        channel_id: &msg.channel_id,
        user_id: &msg.author.id,
        args: command_args.iter().map(|s| s.to_string()).collect(),
    };

    match command_name {
        "channel" => channel::run(&command_context).await,
        "erocheck" => erocheck(dm, ctx, bot, &msg.author.id).await,
        "help" | "たすけて" | "助けて" => help::run(&command_context).await,
        "ping" => ping(dm, ctx).await,
        "calc" => calc(dm, ctx, command_args.join(" "), bot).await,
        "var" => var(dm, ctx, command_args.join(" "), bot).await,
        "varbulk" => varbulk(dm, ctx, command_args.join(" "), bot).await,
        "calcsay" => calcsay(&room_pointer, ctx, command_args.join(" "), bot).await,

        // Unknown command
        _ => {
            dm.say(&ctx.http, "しらないコマンドだよ").await.unwrap();
        }
    }
}

async fn guild_message(bot: &Bot, ctx: &Context, msg: &Message) {
    let channel_id = msg.channel_id;

    // guild内で発言してるってことは確実にmemberなので
    let member = bot
        .guild_id
        .member(&ctx.http, &msg.author.id)
        .await
        .unwrap();

    if let Ok(mut chat_log) = bot
        .chat_log
        .entry(channel_id)
        .or_insert_with(|| Mutex::new(VecDeque::new()))
        .lock()
    {
        if chat_log.len() > 100 {
            chat_log.pop_front();
        }
        chat_log.push_back((member.display_name().to_owned(), msg.clone()));
    }

    if !has_privilege(bot, ctx, msg).await {
        return;
    }

    // if message does not contains any command, ignore
    let command_pattern =
        Regex::new(r"(?ms)((?:まなみ(?:ちゃん)?(?:\s|、|は|って|の)?)|!)(.*)").unwrap();
    let (_prefix, input_string): (String, String) = match command_pattern.captures(&msg.content) {
        Some(caps) => (
            caps.get(1).unwrap().as_str().to_owned(),
            caps.get(2).unwrap().as_str().to_owned(),
        ),
        None => return,
    };

    // dice command
    match parser::parse_dice(&input_string).finish() {
        Ok((_, parsed)) => {
            dice(&msg.channel_id, ctx, parsed).await;
            return;
        }
        Err(Error {
            code: ErrorKind::MapRes,
            ..
        }) => {
            msg.channel_id
                .say(&ctx.http, "数字がおかしいよ")
                .await
                .unwrap();
            return;
        }
        Err(_) => {}
    };

    // handle other command
    let split_message = input_string.split_whitespace().collect::<Vec<&str>>();
    let command_name = split_message[0].trim();
    let command_args = &split_message[1..];
    let reply_channel = &msg.channel_id;

    let command_context = CommandContext {
        bot,
        http: &ctx.http,
        msg,
        guild_id: Some(bot.guild_id),
        channel_id: &msg.channel_id,
        user_id: &msg.author.id,
        args: command_args.iter().map(|s| s.to_string()).collect(),
    };

    match command_name {
        "help" | "たすけて" | "助けて" => {
            help::run(&command_context).await;
        }
        "isprime" => isprime(reply_channel, ctx, command_args).await,
        "calc" => calc(reply_channel, ctx, command_args.join(" "), bot).await,
        "var" => var(reply_channel, ctx, command_args.join(" "), bot).await,
        "varbulk" => varbulk(reply_channel, ctx, command_args.join(" "), bot).await,
        "cclemon" => cclemon(reply_channel, ctx, msg.author.id, command_args).await,
        "jail" => jail_main(reply_channel, ctx, command_args, bot).await,
        "unjail" => unjail_main(reply_channel, ctx, command_args, bot).await,
        "clear" | "全部忘れて" => forget_channel_log(reply_channel, ctx, bot).await,
        // Unknown command
        _ => {
            if msg.content.starts_with('!') {
                reply_channel
                    .say(&ctx.http, "しらないコマンドだよ")
                    .await
                    .unwrap();
            } else {
                // まなみが自由に応答するコーナー
                if reply_channel.get() != bot.channel_ids[4].get() {
                    return;
                }
                #[allow(clippy::or_fun_call)]
                // unwrap_or_else(|_| Mutex::new(VecDeque::new()).lock().unwrap()) とすると、生存期間が合わなくて怒られる
                let query = bot
                    .chat_log
                    .get(&channel_id)
                    .unwrap()
                    .lock()
                    .unwrap_or(Mutex::new(VecDeque::new()).lock().unwrap())
                    .iter()
                    .map(|(name, msg)| ai::Query::from_message(name, &msg.content))
                    .collect();
                let response = bot.ai.generate(query).await;
                let content = match response {
                    Ok(response) => response,
                    Err(e) => e,
                };
                let content = content.replace("うだまなみ: ", "");
                reply_channel.say(&ctx.http, content).await.unwrap();
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

// commands

// change channel
async fn channel(
    reply: &ChannelId,
    ctx: &Context,
    args: &[&str],
    bot: &Bot,
    room_pointer: &ChannelId,
    userid: &UserId,
) {
    // 引数なしの場合はチャンネル一覧を表示
    if args.is_empty() {
        let mut res = MessageBuilder::new();
        res.push("今は")
            .channel(room_pointer)
            .push("で代筆してるよ\n")
            .push("```チャンネル一覧だよ\n");
        for (i, ch) in bot.channel_ids.iter().enumerate() {
            res.push(format!("{i:>2}\t"))
                .push(ch.name(&ctx.http).await.unwrap())
                .push("\n");
        }
        let res = res.push("```").push("使い方: `!channel <ID>`").build();

        reply.say(&ctx.http, &res).await.unwrap();
        return;
    }

    // それ以外の場合は指定されたチャンネルに切り替え
    let Ok(selector) = args[0].parse::<usize>() else {
        reply.say(&ctx.http, "IDは数字で指定してね").await.unwrap();
        return;
    };
    let Some(&next_pointer) = bot.channel_ids.get(selector) else {
        reply
            .say(&ctx.http, "しらないチャンネルだよ")
            .await
            .unwrap();
        return;
    };

    bot.change_room_pointer(userid, next_pointer).unwrap();
    reply
        .say(
            &ctx.http,
            MessageBuilder::new()
                .push("送信先を")
                .channel(next_pointer)
                .push("に設定したよ")
                .build(),
        )
        .await
        .unwrap();
}

// dice command
async fn dice(reply: &ChannelId, ctx: &Context, parsed: parser::Dice) {
    // パース
    let parser::Dice { num, dice, cmp } = parsed;

    // 入力のチェック
    if num > 1000000 {
        reply
            .say(&ctx.http, "そんないっぱい振れないよ")
            .await
            .unwrap();
        return;
    } else if num == 0 {
        reply.say(&ctx.http, "じゃあ振らないよ").await.unwrap();
        return;
    }

    // ダイスロール
    let mut sum = 0;
    let mut vec = vec![];
    for _ in 0..num {
        let r = rand::random::<u64>() % dice + 1;
        vec.push(r.to_string());
        sum += u128::from(r);
    }
    // 結果
    let roll_result = format!("{}D{} -> {}", num, dice, sum);
    // 内訳
    let roll_items = format!(" ({})", vec.join(", "));

    // 比較オプション
    let operation_result = cmp.map(|(operator, operand)| {
        let is_ok = parser::cmp_with_operator(&operator, sum, operand);
        let is_ok = if is_ok { "OK" } else { "NG" };
        format!(" {} {} -> {}", Into::<&str>::into(operator), operand, is_ok)
    });

    // メッセージの生成と送信
    let mut res = MessageBuilder::new();
    res.push(roll_result);
    if 1 < dice && roll_items.len() <= 100 {
        res.push(roll_items);
    }
    if let Some(operation_result) = operation_result {
        res.push(operation_result);
    }
    reply.say(&ctx.http, &res.build()).await.unwrap();
}

// erogaki status check
async fn erocheck(reply: &ChannelId, ctx: &Context, bot: &Bot, user_id: &UserId) {
    let is_erogaki = bot
        .guild_id
        .member(&ctx.http, user_id)
        .await
        .unwrap()
        .roles
        .iter()
        .any(|role| role == &bot.erogaki_role_id);
    let content = if is_erogaki {
        "エロガキ！！！！"
    } else {
        "エロガキじゃないよ"
    };
    reply.say(&ctx.http, content).await.unwrap();
}

async fn isprime(reply: &ChannelId, ctx: &Context, command_args: &[&str]) {
    let [command_args] = command_args else {
        reply
            .say(&ctx.http, "使い方: `!isprime <number>`")
            .await
            .unwrap();
        return;
    };

    let Ok(num) = command_args.parse::<u64>() else {
        reply.say(&ctx.http, "わかんないよ").await.unwrap();
        return;
    };

    let (is_prime, factor) = match num {
        0 | 1 => (false, vec![]),
        2 => (true, vec![2]),
        _ => {
            let mut num = num;
            let mut factor = vec![];

            while num % 2 == 0 {
                num /= 2;
                factor.push(2);
            }

            let mut i = 3;

            while i * i <= num {
                if num % i == 0 {
                    num /= i;
                    factor.push(i);
                } else {
                    i += 2;
                }
            }

            if num != 1 {
                factor.push(num);
            }

            if factor.len() == 1 {
                (true, factor)
            } else {
                (false, factor)
            }
        }
    };

    let is_prime = format!(
        "{}は{}",
        num,
        if is_prime {
            "素数だよ".to_owned()
        } else if factor.is_empty() {
            "素数じゃないよ。あたりまえでしょ？".to_owned()
        } else {
            format!("素数じゃないよ。素因数は{:?}だよ", factor)
        }
    );
    reply.say(&ctx.http, is_prime).await.unwrap();
}

const VAR_DEFAULT: &str = "_";

async fn calc(reply: &ChannelId, ctx: &Context, expression: String, bot: &Bot) {
    var_main(reply, ctx, VAR_DEFAULT.to_owned(), expression, bot).await;
}

async fn var(reply: &ChannelId, ctx: &Context, input: String, bot: &Bot) {
    let split: Vec<&str> = input.split('=').collect();
    let (var, expression) = if split.len() < 2 {
        (VAR_DEFAULT.to_owned(), input)
    } else {
        (split[0].trim().to_owned(), split[1..].join("="))
    };
    var_main(reply, ctx, var, expression, bot).await;
}

async fn calcsay(reply: &ChannelId, ctx: &Context, expression: String, bot: &Bot) {
    let result = calculator::eval_from_str(&expression, &bot.variables);
    if let Ok(result) = result {
        reply.say(&ctx.http, val_as_str(&result)).await.unwrap();
    }
}

async fn varbulk(reply: &ChannelId, ctx: &Context, input: String, bot: &Bot) {
    let code_pattern = Regex::new(r"```[a-zA-Z0-9]*(.*)```").unwrap();

    //get input in code block
    let input = match code_pattern.captures(&input) {
        Some(caps) => caps.get(1).unwrap().as_str().to_owned(),
        None => return,
    };
    let split: Vec<&str> = input.split(';').collect();
    for s in split {
        if s.trim().is_empty() {
            continue;
        }
        var(reply, ctx, s.to_owned(), bot).await;
    }
}

async fn var_main(reply: &ChannelId, ctx: &Context, var: String, expression: String, bot: &Bot) {
    let result = calculator::eval_from_str(&expression, &bot.variables);
    match result {
        Ok(result) => {
            bot.variables.insert(var, result.clone());
            reply.say(&ctx.http, val_as_str(&result)).await.unwrap();
        }
        Err(e) => {
            reply
                .say(&ctx.http, format!("{} ……だってさ。", e))
                .await
                .unwrap();
        }
    }
}

async fn cclemon(reply: &ChannelId, ctx: &Context, author_id: UserId, command_args: &[&str]) {
    let [opponent_id] = command_args else {
        reply
            .say(&ctx.http, "使い方: `!cclemon <相手>`")
            .await
            .unwrap();
        return;
    };
    let Some(opponent_id) = parse_user_mention(opponent_id) else {
        reply
            .say(&ctx.http, "相手をメンションで指定してね")
            .await
            .unwrap();
        return;
    };
    cclemon::cclemon(reply, ctx, (author_id, opponent_id)).await;
}

const JAIL_TERM_MAX: Duration = Duration::from_secs(3600);
const JAIL_TERM_DEFAULT: Duration = Duration::from_secs(15);

async fn jail_main(reply: &ChannelId, ctx: &Context, args: &[&str], bot: &Bot) {
    let (user, jailterm) = match args {
        [user] => {
            let Some(user) = parse_user_mention(user) else {
                reply.say(&ctx.http, "誰？").await.unwrap();
                return;
            };
            (user, JAIL_TERM_DEFAULT)
        }
        [user, args @ ..] => {
            let Some(user) = parse_user_mention(user) else {
                reply.say(&ctx.http, "誰？").await.unwrap();
                return;
            };

            let expression = args.join(" ");

            let Ok(jailtermsec) = calculator::eval_from_str(&expression, &bot.variables) else {
                reply.say(&ctx.http, "刑期がおかしいよ").await.unwrap();
                return;
            };
            let Some(jailtermsec) = calculator::val_as_int(&jailtermsec) else {
                reply.say(&ctx.http, "刑期がおかしいよ").await.unwrap();
                return;
            };
            let Ok(jailtermsec) = u64::try_from(jailtermsec) else {
                reply.say(&ctx.http, "刑期が負だよ").await.unwrap();
                return;
            };

            let jailterm = if jailtermsec > JAIL_TERM_MAX.as_secs() {
                reply
                    .say(
                        &ctx.http,
                        format!(
                            "刑期が長すぎるから切り詰めたよ（最長{}秒）",
                            JAIL_TERM_MAX.as_secs()
                        ),
                    )
                    .await
                    .unwrap();
                JAIL_TERM_MAX
            } else {
                Duration::from_secs(jailtermsec)
            };
            (user, jailterm)
        }
        _ => {
            reply
                .say(&ctx.http, "使い方: `!jail <user> [刑期（秒）]`")
                .await
                .unwrap();
            return;
        }
    };
    jail(reply, ctx, &user, jailterm, bot).await;
}

async fn unjail_main(reply: &ChannelId, ctx: &Context, args: &[&str], bot: &Bot) {
    let [user] = args else {
        reply
            .say(&ctx.http, "使い方: `!unjail <user>`")
            .await
            .unwrap();
        return;
    };
    let Some(user) = parse_user_mention(user) else {
        reply.say(&ctx.http, "誰？").await.unwrap();
        return;
    };

    unjail(
        reply,
        ctx,
        &user,
        &bot.guild_id,
        &[bot.jail_mark_role_id, bot.jail_main_role_id],
        None,
        &bot.jail_process,
    )
    .await;
}

async fn jail(reply: &ChannelId, ctx: &Context, user: &UserId, jailterm: Duration, bot: &Bot) {
    let guild = bot.guild_id;
    let roles = vec![bot.jail_mark_role_id, bot.jail_main_role_id];

    let member = guild.member(&ctx.http, user).await.unwrap();
    if member.add_roles(&ctx.http, &roles).await.is_err() {
        reply.say(&ctx.http, "収監に失敗したよ").await.unwrap();
        return;
    }

    let jail_term_end = Instant::now() + jailterm;
    if let Some((_, end)) = bot.jail_process.get(user).map(|r| *r.value()) {
        if end > jail_term_end {
            let content = format!(
                "{}はすでに収監中だよ（残り刑期：{}秒）",
                user.mention(),
                end.duration_since(Instant::now()).as_secs()
            );
            reply.say(&ctx.http, content).await.unwrap();
            return; // 既に収監中
        } else {
            let content = format!(
                "{}を再収監したよ（残り刑期：{}秒 → {}秒）",
                user.mention(),
                end.duration_since(Instant::now()).as_secs(),
                jailterm.as_secs()
            );

            reply.say(&ctx.http, content).await.unwrap();
        }
    } else {
        let content = format!(
            "{}を収監したよ（刑期：{}秒）",
            user.mention(),
            jailterm.as_secs()
        );

        reply.say(&ctx.http, content).await.unwrap();
    }

    let Some(newid) = bot.jail_id.lock().map_or(None, |mut oldid| {
        *oldid += 1;
        Some(*oldid)
    }) else {
        reply.say(&ctx.http, "再収監に失敗したよ").await.unwrap();
        return;
    };

    let reply = *reply;
    let ctx = ctx.clone();
    let user = *user;
    let roles = roles.to_vec();
    bot.jail_process.insert(user, (newid, jail_term_end));
    let process = bot.jail_process.clone();
    spawn(async move {
        sleep(jailterm).await;
        unjail(&reply, &ctx, &user, &guild, &roles, Some(newid), &process).await;
        drop(process);
    });
}

async fn unjail(
    reply: &ChannelId,
    ctx: &Context,
    user: &UserId,
    guild: &GuildId,
    roles: &[RoleId],
    jail_id: Option<usize>,
    jail_process: &Arc<DashMap<UserId, (usize, Instant)>>,
) {
    let member = guild.member(&ctx.http, user).await.unwrap();

    // 釈放予定表を確認
    if let Some((id, _)) = jail_process.get(user).map(|r| *r.value()) {
        if let Some(jail_id) = jail_id {
            // 釈放するidが指定されている場合
            if jail_id == id {
                //当該idのみ釈放
                jail_process.remove(user);
            } else {
                return; // 釈放しない
            }
        } else {
            // 無条件釈放なら、釈放予定表から削除
            jail_process.remove(user);
        }
    }

    if !member.roles.iter().any(|role| roles.contains(role)) {
        return;
    }

    if member.remove_roles(&ctx.http, roles).await.is_err() {
        reply.say(&ctx.http, "釈放に失敗したよ").await.unwrap();
        return;
    }

    let content = format!("{}を釈放したよ", user.mention());
    reply.say(&ctx.http, content).await.unwrap();
}

async fn forget_channel_log(reply: &ChannelId, ctx: &Context, bot: &Bot) {
    reply.say(&ctx.http, "1……2の……ポカン！").await.unwrap();

    if let Some(reflog) = bot.chat_log.get(reply) {
        if let Ok(mut chat_log) = reflog.lock() {
            chat_log.clear();
        }
    }
}

// ping command
async fn ping(reply: &ChannelId, ctx: &Context) {
    reply.say(&ctx.http, "pong").await.unwrap();
}
