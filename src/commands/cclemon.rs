use crate::cclemon;
use crate::commands::CommandContext;
use serenity::utils::parse_user_mention;

pub async fn run(ctx: &CommandContext<'_>) {
    let [opponent_id] = ctx.args()[..] else {
        ctx.channel_id
            .say(ctx.cache_http(), "使い方: `!cclemon <相手>`")
            .await
            .unwrap();
        return;
    };
    let Some(opponent_id) = parse_user_mention(opponent_id) else {
        ctx.channel_id
            .say(ctx.cache_http(), "相手をメンションで指定してね")
            .await
            .unwrap();
        return;
    };

    cclemon::cclemon(ctx, (ctx.author_id, &opponent_id)).await;
}
