// build.rs

fn main() {
    // 将第二个参数 embed_resource::NONE 加回来
    embed_resource::compile("app.rc", embed_resource::NONE);
}