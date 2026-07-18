#[macro_export]
macro_rules! read_file_err {
    ($result:expr, $path:expr) => {
        $result
            .map_err(|e| miette!(e))
            .wrap_err_with(|| format!("failed to read file `{}`", $path.display()))
    };
}

#[macro_export]
macro_rules! write_file_err {
    ($result:expr, $path:expr) => {
        $result
            .map_err(|e| miette!(e))
            .wrap_err_with(|| format!("failed to write file `{}`", $path.display()))
    };
}

#[macro_export]
macro_rules! remove_file_err {
    ($result:expr, $path:expr) => {
        $result
            .map_err(|e| miette!(e))
            .wrap_err_with(|| format!("failed to remove file `{}`", $path.display()))
    };
}

#[macro_export]
macro_rules! copy_file_err {
    ($result:expr, $from:expr, $to:expr) => {
        $result.map_err(|e| miette!(e)).wrap_err_with(|| {
            format!(
                "failed to copy `{}` to `{}`",
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
                "failed to create symlink `{}` -> `{}`",
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
            .wrap_err_with(|| format!("failed to read symlink `{}`", $path.display()))
    };
}

#[macro_export]
macro_rules! read_dir_err {
    ($result:expr, $path:expr) => {
        $result
            .map_err(|e| miette!(e))
            .wrap_err_with(|| format!("failed to read directory `{}`", $path.display()))
    };
}

#[macro_export]
macro_rules! create_dir_err {
    ($result:expr, $path:expr) => {
        $result
            .map_err(|e| miette!(e))
            .wrap_err_with(|| format!("failed to create directory `{}`", $path.display()))
    };
}

#[macro_export]
macro_rules! remove_dir_err {
    ($result:expr, $path:expr) => {
        $result
            .map_err(|e| miette!(e))
            .wrap_err_with(|| format!("failed to remove directory `{}`", $path.display()))
    };
}

#[macro_export]
macro_rules! parse_err {
    ($result:expr, $path:expr) => {
        $result
            .map_err(|e| miette!(e))
            .wrap_err_with(|| format!("failed to parse file `{}`", $path.display()))
    };
}

#[macro_export]
macro_rules! glob_err {
    ($result:expr, $pattern:expr) => {
        $result
            .map_err(|e| miette!(e))
            .wrap_err_with(|| format!("invalid glob pattern: `{}`", $pattern))
    };
}

#[macro_export]
macro_rules! open_template_err {
    ($result:expr, $path:expr) => {
        $result
            .map_err(|e| miette!(e))
            .wrap_err_with(|| format!("failed to open template `{}`", $path.display()))
    };
}

#[macro_export]
macro_rules! mmap_template_err {
    ($result:expr, $path:expr) => {
        $result
            .map_err(|e| miette!(e))
            .wrap_err_with(|| format!("failed to mmap template `{}`", $path.display()))
    };
}

#[macro_export]
macro_rules! parse_template_err {
    ($result:expr, $path:expr) => {
        $result
            .map_err(|e| miette!(e))
            .wrap_err_with(|| format!("failed to parse template `{}`", $path.display()))
    };
}

#[macro_export]
macro_rules! render_template_err {
    ($result:expr, $path:expr) => {
        $result
            .map_err(|e| miette!(e))
            .wrap_err_with(|| format!("failed to render template `{}`", $path.display()))
    };
}

#[macro_export]
macro_rules! create_file_err {
    ($result:expr, $path:expr) => {
        $result
            .map_err(|e| miette!(e))
            .wrap_err_with(|| format!("failed to create file `{}`", $path.display()))
    };
}
