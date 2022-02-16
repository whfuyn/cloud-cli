mod admin;
mod rpc;
// // mod executor;
// // #[cfg(feature = "evm")]
mod evm;
// mod wallet;
mod key;
mod cldi;
mod bench;

pub use cldi::cldi_cmd;

use crate::crypto::Crypto;
use crate::sdk::context::Context;
use clap::AppFlags;
use clap::AppSettings;
use clap::{App, Arg, ArgMatches};
use std::collections::HashMap;
use std::ffi::OsString;
use tonic::transport::Endpoint;

use anyhow::{anyhow, bail, ensure, Context as _, Result};

use crate::sdk::{
    account::AccountBehaviour, admin::AdminBehaviour, controller::ControllerBehaviour,
    evm::EvmBehaviour, evm::EvmBehaviourExt, executor::ExecutorBehaviour, wallet::WalletBehaviour,
    controller::ControllerClient, executor::ExecutorClient, evm::EvmClient,
    wallet::Wallet
};


/// Command handler that associated with a command.
pub type CommandHandler<'help, Co, Ex, Ev, Wa> =
    // TODO: Is it neccessary to use a &mut Command?
    // No &mut for ArgMatches bc there is no much thing we can do with it.
    fn(&Command<'help, Co, Ex, Ev, Wa>, &ArgMatches, &mut Context<Co, Ex, Ev, Wa>) -> Result<()>;

/// Command
#[derive(Clone)]
pub struct Command<'help, Co, Ex, Ev, Wa> {
    app: App<'help>,
    handler: CommandHandler<'help, Co, Ex, Ev, Wa>,

    subcmds: HashMap<String, Self>,
}

impl<'help, Co, Ex, Ev, Wa> Command<'help, Co, Ex, Ev, Wa> {
    /// Create a new command.
    pub fn new<S: Into<String>>(name: S) -> Self {
        Self {
            app: App::new(name),
            handler: Self::dispatch_subcmd,
            subcmds: HashMap::new(),
        }
    }

    /// (Re)Sets this command's app name.
    pub fn name(mut self, name: &str) -> Self {
        self.app = self.app.name(name);
        self
    }

    pub fn alias<S: Into<&'help str>>(mut self, name: S) -> Self {
        self.app = self.app.alias(name);
        self
    }

    pub fn aliases(mut self, names: &[&'help str]) -> Self {
        self.app = self.app.aliases(names);
        self
    }

    pub fn about<O: Into<Option<&'help str>>>(mut self, about: O) -> Self {
        self.app = self.app.about(about);
        self
    }

    pub fn setting<F: Into<AppFlags>>(mut self, setting: F) -> Self {
        self.app = self.app.setting(setting);
        self
    }

    pub fn arg<A: Into<Arg<'help>>>(mut self, a: A) -> Self {
        self.app = self.app.arg(a);
        self
    }

    pub fn handler(mut self, handler: CommandHandler<'help, Co, Ex, Ev, Wa>) -> Self {
        self.handler = handler;
        self
    }

    /// Add subcommand for this Command.
    pub fn subcommand(mut self, subcmd: Self) -> Self {
        let subcmd_name = subcmd.get_name().to_owned();

        self.app = self.app.subcommand(subcmd.app.clone());
        self.subcmds.insert(subcmd_name, subcmd);

        self
    }

    /// Same as [`subcommand`], but accept multiple subcommands.
    ///
    /// [`Command::subcommand`]: Command::subcommand
    pub fn subcommands<I>(self, subcmds: I) -> Self
    where
        I: IntoIterator<Item = Self>,
    {
        // just a fancy loop!
        subcmds
            .into_iter()
            .fold(self, |this, subcmd| this.subcommand(subcmd))
    }

    pub fn exec(&self, ctx: &mut Context<Co, Ex, Ev, Wa>) -> Result<()> {
        let m = self.app.clone().get_matches();
        self.exec_with(&m, ctx)
    }

    /// Execute this command with context and args.
    pub fn exec_with(
        &self,
        m: &ArgMatches,
        ctx: &mut Context<Co, Ex, Ev, Wa>,
    ) -> Result<()> {
        (self.handler)(self, m, ctx)
    }

    pub fn exec_from<I, T>(&self, iter: I, ctx: &mut Context<Co, Ex, Ev, Wa>) -> Result<()>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let m = self.app.clone().try_get_matches_from(iter)?;
        self.exec_with(&m, ctx)
    }

    pub fn dispatch_subcmd(
        &self,
        m: &ArgMatches,
        ctx: &mut Context<Co, Ex, Ev, Wa>,
    ) -> Result<()> {
        if let Some((subcmd_name, subcmd_matches)) = m.subcommand() {
            if let Some(subcmd) = self.subcmds.get(subcmd_name) {
                subcmd.exec_with(subcmd_matches, ctx)?;
            } else {
                // TODO: this may be an unreachable branch.
                bail!("no subcommand handler for `{}`", subcmd_name);
            }
        }
        Ok(())
    }

    /// Get name of the underlaying clap App.
    pub fn get_name(&self) -> &str {
        self.app.get_name()
    }

    pub fn get_subcommand(&self, subcmd: &str) -> Option<&Self> {
        self.subcmds.get(subcmd)
    }

    pub fn rename_subcommand(&mut self, old: &str, new: &str) -> Result<()> {
        let old_app = self
            .app
            .find_subcommand_mut(old)
            .ok_or(anyhow!("subcommand no found"))?;
        *old_app = old_app.clone().name(new);
        let old_subcmd = self.subcmds.remove(old).expect("subcommand no found");
        self.subcmds.insert(new.into(), old_subcmd.name(new));

        Ok(())
    }

    /// Get matches from the underlaying clap App.
    pub fn get_matches(&self) -> ArgMatches {
        self.app.clone().get_matches()
    }

    // TODO: get matches from

    pub fn get_all_aliases(&self) -> impl Iterator<Item = &str> + '_ {
        self.app.get_all_aliases()
    }
}

// pub fn all_cmd<'help, C: Crypto>() -> Command<'help, ControllerClient, ExecutorClient, EvmClient, Wallet<C>>
// {
//     Command::new("cldi")
//         .about("The command line interface to interact with `CITA-Cloud v6.3.0`.")
//         .arg(
//             Arg::new("controller-addr")
//                 .help("controller address")
//                 .short('r')
//                 .takes_value(true)
//                 // TODO: add validator
//         )
//         .arg(
//             Arg::new("executor-addr")
//                 .help("executor address")
//                 .short('e')
//                 .takes_value(true)
//                 // TODO: add validator
//         )
//         .handler(|cmd, ctx, m| {
//             let rt = ctx.rt.handle().clone();
//             rt.block_on(async {
//                 if let Some(controller_addr) = m.value_of("controller-addr") {
//                     let controller = {
//                         let addr = format!("http://{controller_addr}");
//                         let channel = Endpoint::from_shared(addr)?.connect_lazy();
//                         ControllerClient::new(channel)
//                     };
//                     ctx.controller = controller;
//                 }

//                 if let Some(executor_addr) = m.value_of("executor-addr") {
//                     let executor = {
//                         let addr = format!("http://{executor_addr}");
//                         let channel = Endpoint::from_shared(addr)?.connect_lazy();
//                         ExecutorClient::new(channel)
//                     };

//                     let evm = {
//                         let addr = format!("http://{executor_addr}");
//                         let channel = Endpoint::from_shared(addr).unwrap().connect_lazy();
//                         EvmClient::new(channel)
//                     };

//                     ctx.executor = executor;
//                     ctx.evm = evm;
//                 }
//                 anyhow::Ok(())
//             })
//         })
//         .subcommands([
//             // key::key_cmd(),
//             // admin::admin_cmd(),
//             // // TODO: figure out why I have to specify `C` for this cmd
//             // rpc::rpc_cmd::<C, _, _, _, _>(),
//             // evm::evm_cmd(),
//         ])
// }
