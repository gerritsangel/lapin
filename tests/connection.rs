use lapin::{
    message::DeliveryResult, options::*, publisher_confirm::Confirmation, types::FieldTable,
    BasicProperties, Connection, ConnectionProperties, ConsumerDelegate,
};
use log::info;
use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    },
    thread, time,
};

#[derive(Clone, Debug)]
struct Subscriber {
    hello_world: Arc<AtomicU8>,
}

impl ConsumerDelegate for Subscriber {
    fn on_new_delivery(
        &self,
        delivery: DeliveryResult,
    ) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        let subscriber = self.clone();
        Box::pin(async move {
            println!("received message: {:?}", delivery);

            if let Some(delivery) = delivery.unwrap() {
                println!("data: {}", std::str::from_utf8(&delivery.data).unwrap());

                assert_eq!(delivery.data, b"Hello world!");

                subscriber.hello_world.fetch_add(1, Ordering::SeqCst);
            }
        })
    }
}

#[test]
fn connection() {
    let _ = env_logger::try_init();

    let addr = std::env::var("AMQP_ADDR").unwrap_or_else(|_| "amqp://127.0.0.1:5672/%2f".into());
    let conn = Connection::connect(&addr, ConnectionProperties::default())
        .wait()
        .expect("connection error");

    println!("CONNECTED with configuration: {:?}", conn.configuration());

    //now connected

    //send channel
    let channel_a = conn.create_channel().wait().expect("create_channel");
    //receive channel
    let channel_b = conn.create_channel().wait().expect("create_channel");

    //create the hello queue
    let queue = channel_a
        .queue_declare(
            "hello-async",
            QueueDeclareOptions::default(),
            FieldTable::default(),
        )
        .wait()
        .expect("queue_declare");
    println!("[{}] state: {:?}", line!(), conn.status().state());
    println!("[{}] declared queue: {:?}", line!(), queue);

    //purge the hello queue in case it already exists with contents in it
    let queue = channel_a
        .queue_purge("hello-async", QueuePurgeOptions::default())
        .wait()
        .expect("queue_purge");
    println!("[{}] state: {:?}", line!(), conn.status().state());
    info!("Declared queue {:?}", queue);

    println!("will consume");
    let hello_world = Arc::new(AtomicU8::new(0));
    let subscriber = Subscriber {
        hello_world: hello_world.clone(),
    };
    let consumer = channel_b
        .basic_consume(
            "hello-async",
            "my_consumer",
            BasicConsumeOptions::default(),
            FieldTable::default(),
        )
        .wait()
        .expect("basic_consume");

    println!("will publish");
    let payload = b"Hello world!";
    let confirm = channel_a
        .basic_publish(
            "",
            "hello-async",
            BasicPublishOptions::default(),
            payload.to_vec(),
            BasicProperties::default(),
        )
        .wait()
        .expect("basic_publish")
        .wait()
        .expect("publisher-confirms");
    assert_eq!(confirm, Confirmation::NotRequested);

    consumer.set_delegate(subscriber).expect("set_delegate");
    println!("[{}] state: {:?}", line!(), conn.status().state());

    println!("will publish");
    let confirm = channel_a
        .basic_publish(
            "",
            "hello-async",
            BasicPublishOptions::default(),
            payload.to_vec(),
            BasicProperties::default(),
        )
        .wait()
        .expect("basic_publish")
        .wait()
        .expect("publisher-confirms");
    assert_eq!(confirm, Confirmation::NotRequested);
    println!("[{}] state: {:?}", line!(), conn.status().state());

    thread::sleep(time::Duration::from_millis(100));
    assert_eq!(hello_world.load(Ordering::SeqCst), 2);
}
