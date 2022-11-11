use futures::{future, stream, StreamExt};
use mpl_token_metadata::{
    pda::find_metadata_account,
    state::{Metadata, TokenMetadataAccount},
};
use reqwest_middleware::ClientBuilder;
use reqwest_retry::{policies::ExponentialBackoff, RetryTransientMiddleware};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use solana_account_decoder::UiAccountEncoding;
use solana_client::{
    rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig},
    rpc_filter::{Memcmp, RpcFilterType},
};
use solana_sdk::{commitment_config::CommitmentConfig, pubkey::Pubkey};
use std::{
    collections::{BTreeMap, HashSet},
    env, fs, io,
    thread::sleep,
    time::Duration,
};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct Attribute {
    trait_type: String,
    value: Value,
}

#[serde_with::serde_as]
#[derive(Debug, Deserialize)]
struct JsonMetadata {
    name: String,
    image: String,
    #[serde_as(as = "serde_with::OneOrMany<_>")]
    attributes: Vec<Attribute>,
}

#[serde_with::serde_as]
#[derive(Debug, Serialize)]
struct Token {
    #[serde(rename = "name__")]
    name: String,
    #[serde(rename = "image__")]
    image: String,
    mint_address: String,
    #[serde(flatten)]
    #[serde_as(as = "BTreeMap<_, _>")]
    attributes: Vec<(String, Value)>,
}

impl From<(Pubkey, JsonMetadata)> for Token {
    fn from(
        (
            mint_address,
            JsonMetadata {
                name,
                image,
                attributes,
            },
        ): (Pubkey, JsonMetadata),
    ) -> Self {
        Self {
            name,
            image,
            mint_address: mint_address.to_string(),
            attributes: attributes
                .into_iter()
                .map(|attribute| (attribute.trait_type, attribute.value))
                .collect(),
        }
    }
}

#[derive(Serialize)]
struct TraitsAndTokens {
    collection_name: String,
    trait_types: HashSet<String>,
    tokens: Vec<Token>,
}

#[tokio::main]
async fn main() {
    let mut args = env::args();
    args.next();
    let collection_arg = args.next().expect("missing collection address");
    let collection: Pubkey = collection_arg.parse().expect("invalid collection address");

    let solana_cl = solana_client::nonblocking::rpc_client::RpcClient::new_with_commitment(
        env::var("SOLANA_RPC_URL").unwrap_or_else(|_| {
            env::var("ANCHOR_PROVIDER_URL")
                .expect("neither SOLANA_RPC_URL nor ANCHOR_PROVIDER_URL are set")
        }),
        CommitmentConfig::finalized(),
    );
    let reqwest_cl = ClientBuilder::new(reqwest::Client::new())
        .with(RetryTransientMiddleware::new_with_policy(
            ExponentialBackoff::builder().build_with_max_retries(8),
        ))
        .build();

    println!("fetching collection metadata");
    let collection_data = solana_cl
        .get_account_data(&find_metadata_account(&collection).0)
        .await
        .unwrap();
    let collection_data = Metadata::safe_deserialize(&collection_data).unwrap();
    let collection_name = collection_data.data.name;
    let collection_name =
        collection_name[..collection_name.find('\0').unwrap_or(collection_name.len())].into();

    let mut traits_and_tokens = TraitsAndTokens {
        collection_name,
        trait_types: HashSet::with_capacity(32),
        tokens: Vec::with_capacity(10_000),
    };
    let mut skipped_count = 0usize;

    for offset in [401, 402] {
        println!("fetching on-chain metadata");
        let accounts = loop {
            let accounts = solana_cl
                .get_program_accounts_with_config(
                    &mpl_token_metadata::ID,
                    RpcProgramAccountsConfig {
                        filters: Some(vec![
                            RpcFilterType::DataSize(679),
                            RpcFilterType::Memcmp(Memcmp::new_raw_bytes(
                                offset,
                                collection.to_bytes().to_vec(),
                            )),
                        ]),
                        account_config: RpcAccountInfoConfig {
                            encoding: Some(UiAccountEncoding::Base64),
                            ..RpcAccountInfoConfig::default()
                        },
                        ..RpcProgramAccountsConfig::default()
                    },
                )
                .await;
            match accounts {
                Ok(accounts) => break accounts,
                Err(e) => {
                    eprintln!("Solana RPC returned {e:?}, retrying in 5 secs");
                    sleep(Duration::from_secs(5));
                }
            }
        };

        stream::iter(accounts)
            .map(|(_, acc)| {
                let reqwest_cl = &reqwest_cl;
                async move {
                    let metadata = Metadata::safe_deserialize(&acc.data).unwrap();
                    println!("fetching {}", metadata.data.uri);
                    let response = match reqwest_cl.get(&metadata.data.uri).send().await {
                        Ok(response) => response,
                        Err(e) => {
                            eprintln!("Error fetching {}: {e:?}, skipping", metadata.data.uri);
                            return None;
                        }
                    };
                    match serde_json::from_slice::<JsonMetadata>(&response.bytes().await.unwrap()) {
                        Ok(json_metadata) => Some((metadata.mint, json_metadata)),
                        Err(e) => {
                            eprintln!("Error parsing JSON {}: {e:?}, skipping", metadata.data.uri);
                            None
                        }
                    }
                }
            })
            .buffer_unordered(64)
            .for_each(|token| {
                let (mint_address, json_metadata) =
                    if let Some((mint_address, json_metadata)) = token {
                        (mint_address, json_metadata)
                    } else {
                        skipped_count += 1;
                        return future::ready(());
                    };
                for attribute in &json_metadata.attributes {
                    traits_and_tokens
                        .trait_types
                        .insert(attribute.trait_type.clone());
                }
                traits_and_tokens
                    .tokens
                    .push((mint_address, json_metadata).into());
                future::ready(())
            })
            .await;
    }

    println!(
        "parsed {} tokens, skipped {skipped_count} tokens",
        traits_and_tokens.tokens.len()
    );

    let path = format!("collections/{}.json", collection_arg);
    println!("writing result to {path}");
    match fs::create_dir("collections") {
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        x => x.unwrap(),
    }
    fs::write(
        path,
        serde_json::to_string(&traits_and_tokens)
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
}
