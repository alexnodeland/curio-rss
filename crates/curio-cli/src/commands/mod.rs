//! Command implementations — each one a thin adapter over
//! [`curio_core::CoreHandle`].

mod articles;
mod feeds;
mod init;
mod inspect;
mod transfer;

use std::process::ExitCode;

use crate::app::App;
use crate::cli::{Cli, Command};

/// Opens the profile and dispatches the parsed command.
pub(crate) fn run(cli: Cli) -> anyhow::Result<ExitCode> {
    let mut app = App::open(cli.profile, cli.json)?;
    dispatch(&mut app, cli.command)
}

fn dispatch(app: &mut App, command: Command) -> anyhow::Result<ExitCode> {
    match command {
        Command::Init => init::run(app),
        Command::Feed(command) => feeds::run(app, command),
        Command::Fetch { feed } => feeds::fetch(app, feed.as_deref()),
        Command::List(args) => articles::list(app, &args),
        Command::Show { id } => articles::show(app, &id),
        Command::Open { id } => articles::open(app, &id),
        Command::Star { id } => articles::set_state(app, &id, articles::StateAction::Star),
        Command::Unstar { id } => articles::set_state(app, &id, articles::StateAction::Unstar),
        Command::Later { id } => articles::set_state(app, &id, articles::StateAction::Later),
        Command::Unlater { id } => articles::set_state(app, &id, articles::StateAction::Unlater),
        Command::Archive { id } => articles::set_state(app, &id, articles::StateAction::Archive),
        Command::Unarchive { id } => {
            articles::set_state(app, &id, articles::StateAction::Unarchive)
        }
        Command::Tag { id, tag } => articles::tag(app, &id, &tag, true),
        Command::Untag { id, tag } => articles::tag(app, &id, &tag, false),
        Command::Save { id, dest } => transfer::save(app, &id, dest),
        Command::Dest(command) => transfer::dest(app, command),
        Command::Opml(command) => transfer::opml(app, command),
        Command::Import { file, from } => transfer::import(app, &file, from),
        Command::Events(command) => inspect::events(app, &command),
        Command::Doctor => inspect::doctor(app),
        Command::Search { query, limit } => articles::search(app, &query, limit),
    }
}
