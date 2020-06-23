#![warn(
    rust_2018_idioms,
    deprecated_in_future,
    macro_use_extern_crate,
    missing_debug_implementations,
    unused_qualifications
)]

use anyhow::Error;
use envconfig::Envconfig;
use envconfig_derive::Envconfig;
use futures::prelude::*;
use indoc::indoc;
use serde::Serialize;
use sqlx::{postgres::PgPool, prelude::*};
use warp::{reject, reject::Reject, reply, Filter, Reply};

#[derive(Debug, Envconfig)]
pub struct Config {
    #[envconfig(from = "PORT", default = "8080")]
    pub listen_port: u16,

    #[envconfig(from = "DATABASE_URL")]
    pub database_url: String,

    #[envconfig(from = "MAX_DATABASE_CONNECTIONS", default = "1")]
    pub max_database_connections: u32,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    dotenv::dotenv().ok();
    let config = Config::init().unwrap();

    let pool = PgPool::builder()
        .max_size(config.max_database_connections)
        .build(&config.database_url)
        .await?;

    let fut = warp::serve(routes(pool)).run(([0, 0, 0, 0], config.listen_port));

    println!("Listening on port {}", config.listen_port);
    fut.await;

    Ok(())
}

fn routes(pool: PgPool) -> impl Filter<Extract = impl Reply> + Clone + Send + Sync {
    let pool_clone = pool.clone();
    let categories = warp::path("categories")
        .and(warp::path::end())
        .and_then(move || {
            let pool = pool_clone.clone();
            async move {
                categories(&pool)
                    .await
                    .map(|categories| reply::json(&categories))
                    .map_err(|e| {
                        dbg!(e);
                        reject::custom(ServerError)
                    })
            }
        });

    let reviews = warp::path!("categories" / String / String)
        .and(warp::path::end())
        .and_then(move |category_name: String, leaderboard_name: String| {
            let pool = pool.clone();
            async move {
                reviews(&pool, &category_name, &leaderboard_name)
                    .await
                    .map(|reviews| reply::json(&reviews))
                    .map_err(|e| {
                        dbg!(e);
                        reject::custom(ServerError)
                    })
            }
        });

    let cors = warp::cors().allow_any_origin().build();
    warp::get().and(categories.or(reviews)).with(cors)
}

#[derive(Debug, sqlx::FromRow, Serialize)]
struct CategoryName {
    name: String,
}

async fn categories(pool: &PgPool) -> Result<Vec<String>, Error> {
    let query = indoc!(
        "
        SELECT name
        FROM categories
        "
    );

    let categories: Vec<String> = sqlx::query_as(query)
        .fetch(pool)
        .map_ok(|category_name: CategoryName| category_name.name)
        .try_collect()
        .await?;

    Ok(categories)
}

#[derive(Debug, sqlx::FromRow, Serialize)]
struct Review {
    player_steam_id: i64,
    score: i32,
    is_legal: bool,
}

async fn reviews(
    pool: &PgPool,
    category_name: &str,
    leaderboard_name: &str,
) -> Result<Vec<Review>, Error> {
    let query = indoc!(
        "
        WITH category_id AS
          (SELECT id
           FROM categories
           WHERE name = $1 ),
             leaderboard_id AS
          (SELECT id
           FROM leaderboards
           WHERE name = $2 )
        SELECT player_steam_id,
               score,
               is_legal
        FROM reviews
        WHERE category_id =
            (SELECT *
             FROM category_id)
          AND leaderboard_id =
            (SELECT *
             FROM leaderboard_id)
        ORDER BY player_steam_id
        "
    );

    let reviews: Vec<Review> = sqlx::query_as(query)
        .bind(category_name)
        .bind(leaderboard_name)
        .fetch_all(pool)
        .await?;

    Ok(reviews)
}

#[derive(Debug)]
struct ServerError;

impl Reject for ServerError {}
