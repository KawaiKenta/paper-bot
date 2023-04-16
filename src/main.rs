use std::{cmp::min, env};

use anyhow::Result;
use arxiv::Arxiv;
use rand::seq::SliceRandom;
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Root {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub usage: Usage,
    pub choices: Vec<Choice>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    #[serde(rename = "prompt_tokens")]
    pub prompt_tokens: i64,
    #[serde(rename = "completion_tokens")]
    pub completion_tokens: i64,
    #[serde(rename = "total_tokens")]
    pub total_tokens: i64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    pub message: Message,
    #[serde(rename = "finish_reason")]
    pub finish_reason: String,
    pub index: i64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Body {
    pub model: String,
    pub messages: Vec<Message>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlackMessage {
    pub channel: String,
    pub text: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    // .envファイルを読み込む
    dotenv::dotenv().ok();
    let search_query = env::var("SEARCH_QUERY").expect("SEARCH_QUERY is not set");
    let openai_key = env::var("OPENAI_KEY").expect("SEARCH_QUERY is not set");
    let slack_token = env::var("SLACK_TOKEN").expect("SLACK_TOKEN is not set");
    let slack_channel = env::var("SLACK_CHANNEL").expect("SLACK_CHANNEL is not set");

    // 論文を検索する
    let query = arxiv::ArxivQueryBuilder::new()
        .search_query(&search_query)
        .start(0)
        .max_results(10)
        .sort_by("submittedDate")
        .sort_order("descending")
        .build();
    let mut arxivs = arxiv::fetch_arxivs(query).await?;

    // arxivsからランダムに3つ選ぶ
    arxivs.shuffle(&mut rand::thread_rng());
    for i in 0..min(3, arxivs.len()) {
        let message = translate_paper(&arxivs[i], &openai_key).await;

        // slackに投稿する
        let response = post_to_slack(
            &SlackMessage {
                channel: slack_channel.clone(),
                text: message.unwrap(),
            },
            &slack_token,
        )
        .await;

        match response {
            Ok(_) => println!("🎉 Successfully posted to Slack"),
            Err(e) => println!("{}", e),
        }
    }

    Ok(())
}

async fn post_to_slack(message: &SlackMessage, token: &String) -> Result<String, String> {
    let bearer_auth = format!("Bearer {}", token);
    let url = "https://slack.com/api/chat.postMessage".to_string();

    let client = reqwest::Client::new();
    let response = client
        .post(url)
        .header(ACCEPT, "*/*")
        .header(AUTHORIZATION, bearer_auth)
        .header(CONTENT_TYPE, "application/json")
        .body(serde_json::to_string(&message).unwrap())
        .send()
        .await
        .unwrap();
    match response.status() {
        reqwest::StatusCode::OK => {
            let body = response.text().await.unwrap();
            Ok(body)
        }
        reqwest::StatusCode::UNAUTHORIZED => {
            Err("🛑 Status: UNAUTHORIZED - Need to grab a new token".to_string())
        }
        reqwest::StatusCode::TOO_MANY_REQUESTS => {
            Err("🛑 Status: 429 - Too many requests".to_string())
        }
        _ => Err("🛑 Status: {:#?} - Something unexpected happened".to_string()),
    }
}

async fn translate_paper(arxiv: &Arxiv, key: &String) -> Result<String, String> {
    // TODO:
    let bearer_auth = format!("Bearer {}", key);
    let system = r#"与えられた英語の論文を日本語に訳し、以下のフォーマットで出力してください。
    ```
    タイトル:
    タイトルの日本語訳

    概要:
    概要の日本語訳
    ```
    "#;
    let user = format!("title: {}\nsummary: {}", arxiv.title, arxiv.summary);
    let data: Body = Body {
        model: "gpt-3.5-turbo".to_string(),
        messages: vec![
            Message {
                role: "system".to_string(),
                content: system.to_string(),
            },
            Message {
                role: "user".to_string(),
                content: user.to_string(),
            },
        ],
    };

    let url = "https://api.openai.com/v1/chat/completions".to_string();
    let client = reqwest::Client::new();
    let response = client
        .post(url)
        .header(ACCEPT, "*/*")
        .header(AUTHORIZATION, &bearer_auth)
        .header(CONTENT_TYPE, "application/json")
        .body(serde_json::to_string(&data).unwrap())
        .send()
        .await
        .unwrap();
    match response.status() {
        reqwest::StatusCode::OK => match response.json::<Root>().await {
            Ok(parsed) => {
                let response = format!(
                    "発行日: {}\n{}\n{}\n{}\n",
                    arxiv.published,
                    arxiv.pdf_url,
                    arxiv.title,
                    parsed.choices[0].message.content.to_string()
                );
                Ok(response)
            }
            Err(_) => Err("🛑 Hm, the response didn't match the shape we expected.".to_string()),
        },
        reqwest::StatusCode::UNAUTHORIZED => {
            Err("🛑 Status: UNAUTHORIZED - Need to grab a new token".to_string())
        }
        reqwest::StatusCode::TOO_MANY_REQUESTS => {
            Err("🛑 Status: 429 - Too many requests".to_string())
        }
        _ => Err("🛑 Status: {:#?} - Something unexpected happened".to_string()),
    }
}
