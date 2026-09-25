fn main() {
    // sqlx::migrate! embeds a directory; newly added files must also invalidate
    // Cargo's build cache, not just edits to previously embedded migrations.
    println!("cargo:rerun-if-changed=migrations");
}
