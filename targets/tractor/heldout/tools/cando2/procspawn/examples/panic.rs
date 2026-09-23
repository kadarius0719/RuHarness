use procspawn::{self, spawn};

fn main() {
    procspawn::init();

    let handle = spawn::<_, ()>(
        (),
        |()| {
            panic!("Whatever!");
        },
        None,
    );

    match handle.join() {
        Ok(_) => unreachable!(),
        Err(err) => {
            let panic = err.panic_info().expect("got a non panic error");
            println!("process panicked with {}", panic.message());
            println!("{:#?}", panic);
        }
    }
}
