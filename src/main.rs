use std::convert::Into;
use std::str::FromStr;

use anyhow::Context as _;
use dashmap::DashMap;
use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case},
    character::complete::{space0, u128, u32, u64},
    combinator::{opt, value},
    error::{Error, ErrorKind},
    sequence::{separated_pair, tuple},
    Finish, IResult,
};
use serenity::all::GuildId;
use serenity::async_trait;
use serenity::model::channel::Message;
use serenity::model::gateway::Ready;
use serenity::model::id::{ChannelId, UserId};
use serenity::model::user;
use serenity::prelude::*;
use serenity::utils::MessageBuilder;
use shuttle_runtime::SecretStore;
use tracing::{error, info};

const KOCHIKITE_GUILD_ID: u64 = 1066468273568362496;
const EROGAKI_ROLE_ID: u64 = 1066667753706102824;

struct Bot {
    userdata: DashMap<UserId, UserData>,
    channel_ids: Vec<ChannelId>,
}

struct UserData {
    room_pointer: ChannelId,
    is_erogaki: bool,
}

#[derive(Clone)]
enum CmpOperator {
    Equal,
    GreaterEqual,
    GreaterThan,
    LessEqual,
    LessThan,
    NotEqual,
}

impl Into<&str> for CmpOperator {
    fn into(self) -> &'static str {
        match self {
            CmpOperator::Equal => "==",
            CmpOperator::GreaterEqual => ">=",
            CmpOperator::GreaterThan => ">",
            CmpOperator::LessEqual => "<=",
            CmpOperator::LessThan => "<",
            CmpOperator::NotEqual => "!=",
        }
    }
}

fn cmp_with_operator(operator: &CmpOperator, left: u128, right: u128) -> bool {
    match operator {
        CmpOperator::Equal => left == right,
        CmpOperator::GreaterEqual => left >= right,
        CmpOperator::GreaterThan => left > right,
        CmpOperator::LessEqual => left <= right,
        CmpOperator::LessThan => left < right,
        CmpOperator::NotEqual => left != right,
    }
}

fn parse_cmp_operator(input: &str) -> IResult<&str, CmpOperator> {
    alt((
        value(CmpOperator::Equal, alt((tag("=="), tag("=")))),
        value(CmpOperator::GreaterEqual, tag(">=")),
        value(CmpOperator::GreaterThan, tag(">")),
        value(CmpOperator::LessEqual, tag("<=")),
        value(CmpOperator::LessThan, tag("<")),
        value(CmpOperator::NotEqual, tag("!=")),
    ))(input)
}

fn parse_dice(input: &str) -> IResult<&str, (u32, u64, Option<(CmpOperator, u128)>)> {
    let (remaining_input, output) = tuple((
        separated_pair(u32, tag_no_case("d"), u64),
        opt(tuple((space0, parse_cmp_operator, space0, u128))),
    ))(input)?;
    let ((num, dice), cmp) = output;
    if let Some((_, operator, _, operand)) = cmp {
        Ok((remaining_input, (num, dice, Some((operator, operand)))))
    } else {
        Ok((remaining_input, (num, dice, None)))
    }
}

#[async_trait]
impl EventHandler for Bot {
    async fn message(&self, ctx: Context, msg: Message) {
        // if message is from bot, ignore
        if msg.author.bot {
            return;
        }

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

    async fn ready(&self, _: Context, ready: Ready) {
        info!("{} is connected!", ready.user.name);
    }
}

//direct message
async fn direct_message(bot: &Bot, ctx: &Context, msg: &Message) {
    // ユーザーがこっちにきてはいけないに存在しない場合は無視
    if let Err(_) = GuildId::from(KOCHIKITE_GUILD_ID)
        .member(&ctx.http, &msg.author.id)
        .await
    {
        return;
    }

    // get user data
    let mut user = bot.userdata.entry(msg.author.id).or_insert(UserData {
        room_pointer: bot.channel_ids[0],
        is_erogaki: false,
    });
    // update user data
    let user = update_user(&ctx, &mut user, &msg.author.id).await.unwrap();

    // if message is not command, forward to the room
    if !&msg.content.starts_with("!") {
        if let Err(why) = user.room_pointer.say(&ctx.http, &msg.content).await {
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

    match command_name {
        "channel" => channel(dm, ctx, command_args, &bot.channel_ids, user).await,
        "erocheck" => erocheck(dm, ctx, user.is_erogaki).await,
        "help" => help(dm, ctx, &msg.author.id).await,
        "ping" => ping(dm, ctx).await,

        // Unknown command
        _ => {
            dm.say(&ctx.http, "しらないコマンドだよ").await.unwrap();
        }
    }
}

async fn guild_message(bot: &Bot, ctx: &Context, msg: &Message) {
    // if message is not command, ignore
    if !&msg.content.starts_with("!") {
        return;
    }

    // get user data
    let mut user = bot.userdata.entry(msg.author.id).or_insert(UserData {
        room_pointer: bot.channel_ids[0],
        is_erogaki: false,
    });
    // update user data
    let user = update_user(&ctx, &mut user, &msg.author.id).await.unwrap();

    // dice command
    match parse_dice(&msg.content[1..]).finish() {
        Ok((_, parsed)) => {
            dice(&msg.channel_id, ctx, parsed).await;
            return;
        }
        Err(err) => {
            let Error { input: _, code } = err;
            if let Some(detail) = match code {
                ErrorKind::Digit => Some("数字がおかしいよ"),
                ErrorKind::Alt => Some("`<operator> = <|<=|=|==|!=|>|>=`だよ"),
                _ => None,
            } {
                msg.channel_id.say(&ctx.http, detail).await.unwrap();
                return;
            }
        }
    }

    // handle other command
    let split_message = msg.content.split_whitespace().collect::<Vec<&str>>();
    let command_name = &split_message[0][1..]; // 先頭の "!" を削除
    let command_args = &split_message[1..];
    let reply_channel = &msg.channel_id;

    match command_name {
        "help" => help(reply_channel, ctx, &msg.author.id).await,
        "isprime" => isprime(reply_channel, ctx, command_args).await,
        // Unknown command
        _ => {
            reply_channel
                .say(&ctx.http, "しらないコマンドだよ")
                .await
                .unwrap();
        }
    }
}

async fn update_user<'a>(
    ctx: &Context,
    user: &'a mut UserData,
    author: &UserId,
) -> Result<&'a mut UserData, anyhow::Error> {
    // erogaki role check
    user.is_erogaki = author
        .to_user(&ctx.http)
        .await?
        .has_role(&ctx.http, KOCHIKITE_GUILD_ID, EROGAKI_ROLE_ID)
        .await?;
    Ok(user)
}

// commands

// help command
async fn help(reply: &ChannelId, ctx: &Context, user_id: &UserId) {
    // 文章
    let about_me = "# まなみの自己紹介だよ！\n";
    let about_ghostwrite = "## 代筆機能があるよ！\nまなみは代筆ができるよ！　DMに送ってもらったメッセージを`!channel`で指定されたチャンネルに転送するよ！\n";

    let about_dm = r"## まなみはDMでコマンドを受け付けるよ！
```
!channel        代筆先のチャンネルについてだよ
!erocheck       あなたがエロガキかどうかを判定するよ
!help           このヘルプを表示するよ
!ping           pong!
```
";

    let about_guild = r"## まなみはグループチャットでコマンドを受け付けるよ！
```
![n]d<m>        m面ダイスをn回振るよ
!help           このヘルプを表示するよ
!isprime <n>    nが素数かどうかを判定するよ
```
";

    let mut content = MessageBuilder::new();
    content.push(about_me).push(about_ghostwrite);

    // ユーザーがこっちにきてはいけないに存在する場合はDMの話もする
    if let Ok(_) = GuildId::from(KOCHIKITE_GUILD_ID)
        .member(&ctx.http, user_id)
        .await
    {
        content.push(about_dm);
    }

    content.push(about_guild);
    let content = content.build();

    reply.say(&ctx.http, content).await.unwrap();
}

// change channel
async fn channel(
    reply: &ChannelId,
    ctx: &Context,
    args: &[&str],
    channels: &Vec<ChannelId>,
    user: &mut UserData,
) {
    // 引数なしの場合はチャンネル一覧を表示
    if args.len() == 0 {
        let mut res = MessageBuilder::new();
        res.push("今は")
            .channel(user.room_pointer)
            .push("で代筆してるよ\n")
            .push("```チャンネル一覧だよ\n");
        for (i, ch) in channels.iter().enumerate() {
            res.push(format!("{i:>2}\t"))
                .push(ch.name(&ctx.http).await.unwrap())
                .push("\n");
        }
        let res = res.push("```").push("使い方: `!channel <ID>`").build();

        reply.say(&ctx.http, &res).await.unwrap();
        return;
    }

    // それ以外の場合は指定されたチャンネルに切り替え
    let selector = match args[0].parse::<usize>() {
        Ok(i) => {
            if i >= channels.len() {
                reply
                    .say(&ctx.http, "しらないチャンネルだよ")
                    .await
                    .unwrap();
                return;
            } else {
                i
            }
        }
        Err(_) => {
            reply.say(&ctx.http, "IDは数字で指定してね").await.unwrap();
            return;
        }
    };

    let next_pointer = channels[selector];
    user.room_pointer = next_pointer;
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
async fn dice(reply: &ChannelId, ctx: &Context, parsed: (u32, u64, Option<(CmpOperator, u128)>)) {
    let (num, dice, cmp) = parsed;

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

    let mut sum = 0;
    let mut res = MessageBuilder::new();
    let mut vec = vec![];

    for _ in 0..num {
        let r = rand::random::<u64>() % dice + 1;
        vec.push(r.to_string());
        sum += r as u128;
    }
    res.push(format!("{}D{} -> {}", num, dice, sum));

    let items = format!("({})", vec.join(", "));

    if 1 < dice && items.len() <= 100 {
        res.push(items);
    }

    if cmp.is_none() {
        reply.say(&ctx.http, &res.build()).await.unwrap();
        return;
    }
    let (operator, operand) = cmp.unwrap();

    let is_ok = cmp_with_operator(&operator, sum, operand);
    let is_ok = if is_ok { "OK" } else { "NG" };
    res.push(format!(
        " {} {} -> {}",
        Into::<&str>::into(operator),
        operand,
        is_ok
    ));

    reply.say(&ctx.http, &res.build()).await.unwrap();
}

// erogaki status check
async fn erocheck(reply: &ChannelId, ctx: &Context, is_erogaki: bool) {
    match is_erogaki {
        true => {
            reply.say(&ctx.http, "エロガキ！！！！").await.unwrap();
        }
        false => {
            reply.say(&ctx.http, "エロガキじゃないよ").await.unwrap();
        }
    }
}

async fn isprime(reply: &ChannelId, ctx: &Context, command_args: &[&str]) {
    if command_args.len() != 1 {
        reply
            .say(&ctx.http, "使い方: `!isprime <number>`")
            .await
            .unwrap();
        return;
    }

    let num = if let Ok(num) = command_args[0].parse::<u64>() {
        num
    } else {
        reply.say(&ctx.http, "わかんないよ").await.unwrap();
        return;
    };

    let (is_prime, factor) = match num {
        0 => (false, vec![]),
        1 => (false, vec![]),
        2 => (true, vec![2]),
        _ => {
            let mut num = num;
            let mut i = 2;
            let mut factor = vec![];

            while num % 2 == 0 {
                num /= 2;
                factor.push(2);
            }

            i = 3;

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
        } else {
            if factor.len() == 0 {
                format!("素数じゃないよ。あたりまえでしょ？")
            } else {
                format!("素数じゃないよ。素因数は{:?}だよ", factor)
            }
        }
    );
    reply.say(&ctx.http, is_prime).await.unwrap();
}

// ping command
async fn ping(reply: &ChannelId, ctx: &Context) {
    reply.say(&ctx.http, "pong").await.unwrap();
}

#[shuttle_runtime::main]
async fn serenity(
    #[shuttle_runtime::Secrets] secrets: SecretStore,
) -> shuttle_serenity::ShuttleSerenity {
    // Get the discord token set in `Secrets.toml`
    let token = secrets
        .get("DISCORD_TOKEN")
        .context("'DISCORD_TOKEN' was not found")?;

    // Set gateway intents, which decides what events the bot will be notified about
    let intents = GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT
        | GatewayIntents::DIRECT_MESSAGES;

    let userdata = DashMap::new();
    let channel_ids = vec![
        secrets.get("FREETALK1_ROOM_ID"),
        secrets.get("FREETALK2_ROOM_ID"),
        secrets.get("MADSISTERS_ROOM_ID"),
        secrets.get("SHYBOYS_ROOM_ID"),
        secrets.get("DEBUG_ROOM_ID"),
    ]
    .into_iter()
    .map(|id| ChannelId::from_str(&id.unwrap()).unwrap())
    .collect();

    let client = Client::builder(&token, intents)
        .event_handler(Bot {
            userdata,
            channel_ids,
        })
        .await
        .expect("Err creating client");

    Ok(client.into())
}
