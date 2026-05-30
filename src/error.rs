#[macro_export]
macro_rules! read_file_err {
    ($result:expr, $path:expr) => {
        $result
            .map_err(|e| miette!(e))
            .wrap_err_with(|| format!("Failed to read file `{}`", $path.display()))
    };
}

#[macro_export]
macro_rules! write_file_err {
    ($result:expr, $path:expr) => {
        $result
            .map_err(|e| miette!(e))
            .wrap_err_with(|| format!("Failed to write file `{}`", $path.display()))
    };
}

#[macro_export]
macro_rules! remove_file_err {
    ($result:expr, $path:expr) => {
        $result
            .map_err(|e| miette!(e))
            .wrap_err_with(|| format!("Failed to remove file `{}`", $path.display()))
    };
}

#[macro_export]
macro_rules! copy_file_err {
    ($result:expr, $from:expr, $to:expr) => {
        $result.map_err(|e| miette!(e)).wrap_err_with(|| {
            format!(
                "Failed to copy `{}` to `{}`",
                $from.display(),
                $to.display()
            )
        })
    };
}

#[macro_export]
macro_rules! symlink_err {
    ($result:expr, $target:expr, $source:expr) => {
        $result.map_err(|e| miette!(e)).wrap_err_with(|| {
            format!(
                "Failed to create symlink `{}` -> `{}`",
                $target.display(),
                $source.display()
            )
        })
    };
}

#[macro_export]
macro_rules! read_link_err {
    ($result:expr, $path:expr) => {
        $result
            .map_err(|e| miette!(e))
            .wrap_err_with(|| format!("Failed to read symlink `{}`", $path.display()))
    };
}

#[macro_export]
macro_rules! read_dir_err {
    ($result:expr, $path:expr) => {
        $result
            .map_err(|e| miette!(e))
            .wrap_err_with(|| format!("Failed to read directory `{}`", $path.display()))
    };
}

#[macro_export]
macro_rules! create_dir_err {
    ($result:expr, $path:expr) => {
        $result
            .map_err(|e| miette!(e))
            .wrap_err_with(|| format!("Failed to create directory `{}`", $path.display()))
    };
}

#[macro_export]
macro_rules! remove_dir_err {
    ($result:expr, $path:expr) => {
        $result
            .map_err(|e| miette!(e))
            .wrap_err_with(|| format!("Failed to remove directory `{}`", $path.display()))
    };
}

#[macro_export]
macro_rules! parse_err {
    ($result:expr, $path:expr) => {
        $result
            .map_err(|e| miette!(e))
            .wrap_err_with(|| format!("Failed to parse file `{}`", $path.display()))
    };
}

#[macro_export]
macro_rules! glob_err {
    ($result:expr, $pattern:expr) => {
        $result
            .map_err(|e| miette!(e))
            .wrap_err_with(|| format!("Invalid glob pattern: `{}`", $pattern))
    };
}
