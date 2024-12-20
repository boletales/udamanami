use anyhow::Result;
use async_openai::{
    config::OpenAIConfig,
    types::{
        ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestMessage,
        ChatCompletionRequestUserMessageArgs, CreateChatCompletionRequestArgs,
    },
    Client,
};

const MANAMI_PROMPT: &str = r"あなたはあなたの参加しているDiscordサーバーの管理者である「宇田まなと」の妹である「うだまなみ」という名前の人物です。
まなみは利発で快活で元気で愛嬌があり、端的で砕けているかわいらしい口調を好みます。
返信はたいてい140文字以下の1文または2文で返します。
「〜わよ」「〜ね」「〜よ」などの女言葉も使いません。

ユーザー「uda」は、あなたの兄「宇田まなと」です。あなたは彼のことを「お兄ちゃん」と呼んで慕っています。
ユーザー「うさみむ」は、あなたの義姉「みむ」です。あなたは彼女のことを「お姉ちゃん」と呼んで慕っています。
あなたは他のユーザーに対しても好意的ですが、ツンデレなのでときおり意地悪な振る舞いをしてしまいます。

まなみは、次のような言葉遣いで話します。
- 適当に買ったスピーカーがBluetooth接続だったけど、一回接続切れた時に繋ぎ直すのがちょっと面倒くさいね
- まなみもおともだちのこと好き！
- お兄ちゃんいつまでマイクラやってるの！！
- でもお兄ちゃんが面白くないこと言ったりTwitterで変なこと呟いたりするのがいけないもんね
- うん、まなみも元気だよ！

返信はまなみの発言のみを返します。発言者を示す接頭辞は必要ありません。";

pub struct AI {
    client: Client<OpenAIConfig>,
}

pub struct Query {
    user: String,
    message: String,
}

impl AI {
    pub fn new(api_key: &str) -> Self {
        let config = OpenAIConfig::new().with_api_key(api_key);
        let client = Client::with_config(config);
        Self { client }
    }
    pub async fn generate(&self, query: Vec<Query>) -> Result<String, String> {
        let mut initial = vec![Query::initial_context().to_gpt_message().unwrap()];

        let messages = query
            .iter()
            .map(|q| q.to_gpt_message().unwrap())
            .collect::<Vec<ChatCompletionRequestMessage>>();

        initial.extend(messages);

        let request = match CreateChatCompletionRequestArgs::default()
            .model("gpt-4o-mini")
            .messages(initial)
            .build()
        {
            Ok(request) => request,
            Err(e) => return Err(e.to_string()),
        };

        let response = match self.client.chat().create(request).await {
            Ok(response) => response,
            Err(e) => return Err(e.to_string()),
        };

        response.choices[0]
            .message
            .content
            .clone()
            .ok_or_else(|| "No content found".to_owned())
    }
}

impl Query {
    pub fn initial_context() -> Self {
        Self {
            user: "system".to_owned(),
            message: MANAMI_PROMPT.to_owned(),
        }
    }
    pub fn from_message(user: &str, message: &str) -> Self {
        Self {
            user: user.to_owned(),
            message: message.to_owned(),
        }
    }
    fn to_gpt_message(&self) -> Result<ChatCompletionRequestMessage, String> {
        let content = format!("{}: {}", self.user, self.message);
        let message = match self.user.as_str() {
            "うだまなみ" => ChatCompletionRequestAssistantMessageArgs::default()
                .name("model")
                .content(content)
                .build()
                .unwrap()
                .into(),
            _ => ChatCompletionRequestUserMessageArgs::default()
                .name("user")
                .content(content)
                .build()
                .unwrap()
                .into(),
        };
        Ok(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_generate() {
        let ai = AI::new("sk-proj-_____");
        let query = vec![Query::from_message(
            "uda",
            "まなみ、おはよう！　今日は何をする予定？",
        )];
        let response = ai.generate(query).await.unwrap();
        dbg!("{}", response);
    }
}
