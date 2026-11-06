//! 右クリックメニューの組み立て時間を測る。

use std::path::Path;
use std::time::Instant;

fn main() {
    filer_shell::init_thread();
    let path = std::env::args().nth(1).expect("file");
    let path = Path::new(&path);
    let (folder, name) = (path.parent().unwrap(), path.file_name().unwrap().to_os_string());
    for round in 1..=3 {
        let start = Instant::now();
        let result = filer_shell::preload_items_menu(folder, std::slice::from_ref(&name));
        println!("round {round}: {} ms {result:?}", start.elapsed().as_millis());
    }
}
