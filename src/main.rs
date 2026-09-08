use clap::CommandFactory;
use onenote_cli::{
    cli::{Cli, Command, Format, Notebooks, PageArgs, Pages, Sections},
    desktop,
    error::Error,
    model, output, schema, write,
};
use serde_json::{Value, json};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut parser = Cli::command();
    if args
        .iter()
        .take_while(|a| a.as_str() != "--")
        .any(|a| a == "--no-color")
        || std::env::var_os("NO_COLOR").is_some()
    {
        parser = parser.color(clap::ColorChoice::Never);
    }
    use clap::FromArgMatches;
    let cli = match parser
        .try_get_matches_from(&args)
        .and_then(|m| Cli::from_arg_matches(&m))
    {
        Ok(cli) => cli,
        Err(e)
            if matches!(
                e.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) =>
        {
            e.exit()
        }
        Err(e) => {
            let error = Error::invalid(e.to_string());
            output::error(&error, output::format_from_args(&args));
            std::process::exit(error.code());
        }
    };
    let format = cli.output;
    if let Err(e) = dispatch(cli).await {
        output::error(&e, format);
        std::process::exit(e.code());
    }
}
async fn list(
    operation: &str,
    scope: Option<&str>,
    query: Option<&str>,
    kind: &str,
    page: PageArgs,
) -> Result<Value, Error> {
    model::validate_fields(&page)?;
    let xml = desktop::run(json!({"operation":operation,"scope":scope,"query":query})).await?;
    if operation == "search" {
        model::search(&xml, &page)
    } else {
        model::collection(&xml, kind, &page)
    }
}
async fn dispatch(cli: Cli) -> Result<(), Error> {
    let value = match cli.command {
        None => {
            if output::structured(cli.output) {
                return Err(Error::invalid(
                    "No command provided; run `onenote --help` or `onenote schema`",
                ));
            }
            let mut help = Cli::command();
            if cli.no_color || std::env::var_os("NO_COLOR").is_some() {
                help = help.color(clap::ColorChoice::Never);
            }
            help.print_help()?;
            return Ok(());
        }
        Some(Command::Notebooks {
            command: Notebooks::List(page),
        }) => list("notebooks", None, None, "Notebook", page).await?,
        Some(Command::Sections {
            command: Sections::List { notebook, page },
        }) => list("sections", Some(&notebook), None, "Section", page).await?,
        Some(Command::Pages {
            command: Pages::List { section, page },
        }) => list("pages", Some(&section), None, "Page", page).await?,
        Some(Command::Pages {
            command: Pages::Read { id, xml },
        }) => {
            let content = desktop::run(json!({"operation":"read","id":id})).await?;
            model::read(&content, &id, xml)?
        }
        Some(Command::Pages {
            command:
                Pages::Create {
                    section,
                    title,
                    content,
                },
        }) => write_page("create", &section, Some(&title), content).await?,
        Some(Command::Pages {
            command: Pages::Append { id, content },
        }) => write_page("append", &id, None, content).await?,
        Some(Command::Search { query, scope, page }) => {
            list("search", scope.as_deref(), Some(&query), "Page", page).await?
        }
        Some(Command::Doctor { offline }) => {
            if offline {
                let check = desktop::powershell();
                json!({"available":null,"verified":false,"platform_supported":check.is_ok(),"message":match check {Ok(_) => "Windows PowerShell found. Run `onenote doctor` to verify OneNote COM access.".into(), Err(e) => e.message}})
            } else {
                let xml = desktop::run(json!({"operation":"probe"})).await?;
                let notebooks = model::collection(
                    &xml,
                    "Notebook",
                    &PageArgs {
                        limit: 1,
                        offset: 0,
                        fields: vec![],
                    },
                )?;
                json!({"available":true,"verified":true,"notebook_count":notebooks["total"],"message":format!("Connected through Windows PowerShell. {} open notebooks. No separate sign-in required.",notebooks["total"])})
            }
        }
        Some(Command::Capabilities) => schema::capabilities(),
        Some(Command::Schema { command }) => {
            let value = schema::generate(command.as_deref());
            if value["commands"].as_array().is_some_and(Vec::is_empty) {
                return Err(Error::new(
                    "not_found",
                    "Command is not declared; run `onenote schema`",
                ));
            }
            return output::print(&value, Format::Json, true);
        }
        Some(Command::Completions { shell }) => {
            clap_complete::generate(
                shell,
                &mut Cli::command(),
                "onenote",
                &mut std::io::stdout(),
            );
            return Ok(());
        }
    };
    output::print(&value, cli.output, cli.no_color)
}

async fn write_page(
    operation: &str,
    target: &str,
    title: Option<&str>,
    content: onenote_cli::cli::ContentArgs,
) -> Result<Value, Error> {
    let mut request = write::prepare(operation, target, title, &content)?;
    if content.dry_run {
        request["dry_run"] = json!(true);
        return Ok(request);
    }
    let result = desktop::write(request).await?;
    let id = result["id"].as_str().ok_or_else(|| {
        Error::new(
            "write_uncertain",
            "Write response has no page ID. Inspect OneNote before retrying.",
        )
    })?;
    let xml = result["xml"].as_str().ok_or_else(|| {
        Error::new(
            "write_uncertain",
            format!("Write response has no page content. Inspect page {id} before retrying."),
        )
        .with_page(id)
    })?;
    let mut page = model::read(xml, id, false).map_err(|e| {
        Error::new(
            "write_uncertain",
            format!(
                "Cannot verify page {id}: {}. Inspect it before retrying.",
                e.message
            ),
        )
        .with_page(id)
    })?;
    if result["verified"] != true {
        return Err(Error::new(
            "write_uncertain",
            format!("Write was not verified. Inspect page {id} before retrying."),
        )
        .with_page(id));
    }
    page["operation"] = json!(operation);
    page["verified"] = json!(true);
    Ok(page)
}
