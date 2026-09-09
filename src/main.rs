use clap::CommandFactory;
use onenote_cli::{
    cli::{
        Cli, Command, ConfigCommand, Format, Notebooks, PageArgs, Pages, ProfileCommand, Sections,
    },
    config::{self, Store},
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
    selected: Option<&str>,
    operation: &str,
    scope: Option<&str>,
    query: Option<&str>,
    kind: &str,
    page: PageArgs,
) -> Result<Value, Error> {
    model::validate_fields(&page)?;
    let (_, profile) = Store::open()?.resolve(selected)?;
    let xml = desktop::run(
        &profile,
        json!({"operation":operation,"scope":scope,"query":query}),
    )
    .await?;
    if operation == "search" {
        model::search(&xml, &page)
    } else {
        model::collection(&xml, kind, &page)
    }
}
async fn dispatch(cli: Cli) -> Result<(), Error> {
    let selected = cli.profile.as_deref();
    let resolve = || Store::open()?.resolve(selected);
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
        }) => list(selected, "notebooks", None, None, "Notebook", page).await?,
        Some(Command::Sections {
            command: Sections::List { notebook, page },
        }) => list(selected, "sections", Some(&notebook), None, "Section", page).await?,
        Some(Command::Pages {
            command: Pages::List { section, page },
        }) => list(selected, "pages", Some(&section), None, "Page", page).await?,
        Some(Command::Pages {
            command: Pages::Read { id, xml },
        }) => {
            let content = desktop::run(&resolve()?.1, json!({"operation":"read","id":id})).await?;
            model::read(&content, &id, xml)?
        }
        Some(Command::Pages {
            command:
                Pages::Create {
                    section,
                    title,
                    content,
                },
        }) => write_page(selected, "create", &section, Some(&title), content).await?,
        Some(Command::Pages {
            command: Pages::Append { id, content },
        }) => write_page(selected, "append", &id, None, content).await?,
        Some(Command::Search {
            query,
            scope,
            scan,
            scan_limit,
            scan_offset,
            page,
        }) => {
            if scan {
                model::validate_fields(&page)?;
                let result = desktop::scan(&resolve()?.1, json!({"operation":"scan", "scope":scope,
                    "scan_limit":scan_limit.unwrap_or(100), "scan_offset":scan_offset.unwrap_or(0)})).await?;
                model::scan(&result, &query, &page)?
            } else {
                list(
                    selected,
                    "search",
                    scope.as_deref(),
                    Some(&query),
                    "Page",
                    page,
                )
                .await?
            }
        }
        Some(Command::Init(args)) => {
            let store = Store::open()?;
            let name = selected.unwrap_or("default");
            store.ensure_available(name, args.force)?;
            let profile = config::Profile::from_args(&args)?;
            if !args.no_check {
                desktop::run(&profile, json!({"operation":"probe"})).await?;
            }
            store.save(name, profile.clone(), args.force)?;
            json!({"view":"init","profile":name,"connection":profile,"path":store.path,"verified":!args.no_check,"target":profile.target(),"message":if args.no_check {"Profile saved and selected. Connection not checked."} else {"Profile saved and selected. OneNote connection verified."}})
        }
        Some(Command::Profile { command }) => {
            let store = Store::open()?;
            match command {
                ProfileCommand::List => {
                    let config = store.read()?.unwrap_or_default();
                    let profiles: Vec<_> = config.profiles.iter().map(|(name,p)| json!({"name":name,"active":config.active_profile.as_ref() == Some(name),"selected":selected.map_or(config.active_profile.as_ref() == Some(name), |s| s == name),"backend":p.backend,"target":p.target(),"read_only":p.read_only})).collect();
                    json!({"view":"profiles","profiles":profiles,"active_profile":config.active_profile,"path":store.path})
                }
                ProfileCommand::Use { name } => {
                    store.update(|c| {
                        if !c.profiles.contains_key(&name) {
                            return Err(Error::new(
                                "config_error",
                                format!(
                                    "Profile '{name}' does not exist; run `onenote profile list`"
                                ),
                            ));
                        }
                        c.active_profile = Some(name.clone());
                        Ok(())
                    })?;
                    json!({"view":"profile_change","profile":name,"message":format!("Active profile: {name}"),"override":selected})
                }
                ProfileCommand::Remove { name } => {
                    store.update(|c| {
                        if c.profiles.remove(&name).is_none() {
                            return Err(Error::new(
                                "config_error",
                                format!("Profile '{name}' does not exist"),
                            ));
                        }
                        if c.active_profile.as_deref() == Some(&name) {
                            c.active_profile = None;
                        }
                        Ok(())
                    })?;
                    json!({"view":"profile_change","profile":name,"message":format!("Removed profile: {name}"),"override":selected})
                }
            }
        }
        Some(Command::Config {
            command: ConfigCommand::Path,
        }) => json!({"view":"config_path","path":config::path()?}),
        Some(Command::Config {
            command: ConfigCommand::Show,
        }) => {
            let (name, profile) = resolve()?;
            json!({"view":"config","profile":name,"source":if selected.is_some() {"override"} else if name.is_some() {"active_profile"} else {"implicit_desktop"},"connection":profile,"path":config::path()?,"target":profile.target()})
        }
        Some(Command::Doctor { offline }) => {
            let (name, profile) = resolve()?;
            if offline {
                let check = desktop::executable(&profile);
                json!({"available":null,"verified":false,"profile":name,"backend":profile.backend,"target":profile.target(),"platform_supported":check.is_ok(),"message":match check {Ok(_) => format!("Transport available for {}. Run `onenote doctor` to verify OneNote access.", profile.target()), Err(e) => e.message}})
            } else {
                let xml = desktop::run(&profile, json!({"operation":"probe"})).await?;
                let notebooks = model::collection(
                    &xml,
                    "Notebook",
                    &PageArgs {
                        limit: 1,
                        offset: 0,
                        fields: vec![],
                    },
                )?;
                json!({"available":true,"verified":true,"profile":name,"backend":profile.backend,"target":profile.target(),"notebook_count":notebooks["total"],"message":format!("Connected to {}. {} open notebooks. No separate OneNote sign-in required.",profile.target(),notebooks["total"])})
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
    selected: Option<&str>,
    operation: &str,
    target: &str,
    title: Option<&str>,
    content: onenote_cli::cli::ContentArgs,
) -> Result<Value, Error> {
    let mut request = write::prepare(operation, target, title, &content)?;
    let (name, profile) = Store::open()?.resolve(selected)?;
    if content.dry_run {
        request["profile"] = json!(name);
        request["connection"] = json!(profile);
        request["dry_run"] = json!(true);
        return Ok(request);
    }
    let result = desktop::write(&profile, request).await?;
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
