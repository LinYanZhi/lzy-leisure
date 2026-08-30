fn main() {
    let attrs = tauri_build::Attributes::new().plugin(
        "appkit-core",
        tauri_build::InlinedPlugin::new()
            .commands(&[
                "delete_data_files",
                "list_data_files",
                "read_all_local_files",
                "list_database_files",
                "open_directory",
                "check_path_exists",
                "save_file",
                "get_db_tables",
                "clear_db_table",
            ])
            .default_permission(tauri_build::DefaultPermissionRule::AllowAllCommands),
    );
    tauri_build::try_build(attrs).unwrap()
}
