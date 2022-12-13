use std::panic;

use actix_mysql::{
    configuration::{get_configuration, DatabaseSettings},
    telemetry::{get_subscriber, init_subscriber},
    startup::{get_connection_pool, Application},
};
use sqlx::{MySqlPool, MySqlConnection, Connection, Executor};
use once_cell::sync::Lazy;


pub struct TestApp {
    pub address: String,
    pub db_pool: MySqlPool,
    pub db_name: String,
    pub port: u16,
    pub api_client: reqwest::Client,
    pub api_key: String,
    pub cookie: String,
}

impl TestApp {
    pub async fn post_cupom(&self, body: serde_json::Value) -> reqwest::Response {
        return self.api_client
            .post(&format!("{}/cupom", &self.address))
            .header("Cookie", &self.cookie)
            .json(&body)
            .send()
            .await
            .expect("Failed to execute request");
    }

    pub async fn authenticate_request(&self) -> reqwest::Response {
        return self.api_client
        .post(&format!("{}/authenticate", &self.address))
        .json(&serde_json::json!({
            "api_key": &self.api_key
        }))
        .send()
        .await
        .expect("Failed to execute request.");
    }
}


// Ensure that the `tracing` stack is only initialised once using `once_cell`
static TRACING: Lazy<()> = Lazy::new(|| {
    let default_filter_level = "trace".to_string();
    let subscriber_name = "test".to_string();

    // We cannot assign the output of `get_subscriber` to a variable based on the value
    // of `TEST_LOG` because the sink is part of the type returned by `get_subscriber`,
    // therefore they are not the same type. We could work around it, but this is the
    // most straight-forward way of moving forward.
    if std::env::var("TEST_LOG").is_ok() {
        let subscriber = get_subscriber(subscriber_name, default_filter_level, std::io::stdout);
        init_subscriber(subscriber);
    } else {
        let subscriber = get_subscriber(subscriber_name, default_filter_level, std::io::sink);
        init_subscriber(subscriber);
    };
});

pub async fn spawn_app() -> TestApp {
    // The first time `initialize` is invoked the code in `TRACING` is executed.
    // All other invocations will instead skip execution.
    Lazy::force(&TRACING);

    // Randomise configuration to ensure test isolation
    let configuration = {
        let mut c = get_configuration().expect("Failed to read configuration.");
        // Use a random OS port
        c.application.port = 0;
        c
    };

    // Create and migrate the database
    configure_test_database(&configuration.database).await;

    // Launch the application as a background task
    let application = Application::build(configuration.clone(), true)
        .await
        .expect("Failed to build application.");
    let application_port = application.port();

    // Get the port before spawning the application
    let address = format!("http://127.0.0.1:{}", application.port());
    let _ = tokio::spawn(application.run_until_stopped());

    let response = reqwest::Client::new()
        .post(&format!("{}/authenticate", &address))
        .json(&serde_json::json!({
            "api_key": &configuration.application.api_key
        }))
        .send()
        .await
        .expect("Failed to execute request.");

    
    let cookie =  response.headers().get("Set-Cookie").unwrap().to_str().unwrap();
    let unsecure_cookie = cookie.replace(" Secure", "");

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .cookie_store(true)
        .build()
        .unwrap();

    return TestApp {
        address,
        port: application_port,
        db_pool: get_connection_pool(&configuration.database, true),
        db_name: configuration.database.test_db_name,
        api_client: client,
        api_key: configuration.application.api_key,
        cookie: unsecure_cookie,
    };
}

pub async fn configure_test_database(config: &DatabaseSettings) -> MySqlPool {
    // Create database
    let mut connection = MySqlConnection::connect_with(&config.without_db())
        .await
        .expect("Failed to connect to database.");

    if (!config.test_db_name.contains("TEST")){
        panic!("`TEST` string not found in Test Database name, is it correct?");
    }

    let _ = connection
        .execute(format!(r#"DROP DATABASE IF EXISTS {};"#, config.test_db_name).as_str())
        .await
        .unwrap();
        
    connection
        .execute(format!(r#"CREATE DATABASE IF NOT EXISTS {};"#, config.test_db_name).as_str())
        // .execute(format!(r#"CREATE DATABASE test_rust;"#).as_str())
        .await
        .expect("Failed to create test database.");
    
    // Migrate database
    let connection_pool = MySqlPool::connect_with(config.with_db(true))
        .await
        .expect("Failed to connect to test database.");
    let _ = sqlx::migrate!("./migrations")
        .run(&connection_pool)
        .await;
        // .expect("Failed to migrate the test database.");

    return connection_pool;
}