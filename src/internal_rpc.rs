use crate::{
    channels::Channels, executor::Executor, socket_state::SocketStateHandle, types::ShortUInt,
    Error, Result,
};
use crossbeam_channel::{Receiver, Sender};
use log::trace;
use std::{future::Future, sync::Arc};

pub(crate) struct InternalRPC {
    rpc: Receiver<InternalCommand>,
    handle: InternalRPCHandle,
}

#[derive(Clone)]
pub(crate) struct InternalRPCHandle {
    sender: Sender<InternalCommand>,
    waker: SocketStateHandle,
    executor: Arc<dyn Executor>,
}

impl InternalRPCHandle {
    pub(crate) fn close_channel(&self, channel_id: u16, reply_code: ShortUInt, reply_text: String) {
        self.send(InternalCommand::CloseChannel(
            channel_id, reply_code, reply_text,
        ));
    }

    pub(crate) fn close_connection(
        &self,
        reply_code: ShortUInt,
        reply_text: String,
        class_id: ShortUInt,
        method_id: ShortUInt,
    ) {
        self.send(InternalCommand::CloseConnection(
            reply_code, reply_text, class_id, method_id,
        ));
    }

    pub(crate) fn send_connection_close_ok(&self, error: Error) {
        self.send(InternalCommand::SendConnectionCloseOk(error));
    }

    pub(crate) fn remove_channel(&self, channel_id: u16, error: Error) {
        self.send(InternalCommand::RemoveChannel(channel_id, error));
    }

    pub(crate) fn set_connection_closing(&self) {
        self.send(InternalCommand::SetConnectionClosing);
    }

    pub(crate) fn set_connection_closed(&self, error: Error) {
        self.send(InternalCommand::SetConnectionClosed(error));
    }

    pub(crate) fn set_connection_error(&self, error: Error) {
        self.send(InternalCommand::SetConnectionError(error));
    }

    fn send(&self, command: InternalCommand) {
        trace!("Queuing internal RPC command: {:?}", command);
        // The only scenario where this can fail if this is the IoLoop already exited
        let _ = self.sender.send(command);
        self.waker.wake();
    }

    pub(crate) fn register_internal_future(
        &self,
        f: impl Future<Output = Result<()>> + Send + 'static,
    ) -> Result<()> {
        let internal_rpc = self.clone();
        self.executor.spawn(Box::pin(async move {
            if let Err(err) = f.await {
                internal_rpc.set_connection_error(err);
            }
        }))
    }
}

#[derive(Debug)]
enum InternalCommand {
    CloseChannel(u16, ShortUInt, String),
    CloseConnection(ShortUInt, String, ShortUInt, ShortUInt),
    SendConnectionCloseOk(Error),
    RemoveChannel(u16, Error),
    SetConnectionClosing,
    SetConnectionClosed(Error),
    SetConnectionError(Error),
}

impl InternalRPC {
    pub(crate) fn new(executor: Arc<dyn Executor>, waker: SocketStateHandle) -> Self {
        let (sender, rpc) = crossbeam_channel::unbounded();
        let handle = InternalRPCHandle {
            sender,
            waker,
            executor,
        };
        Self { rpc, handle }
    }

    pub(crate) fn handle(&self) -> InternalRPCHandle {
        self.handle.clone()
    }

    pub(crate) fn poll(&self, channels: &Channels) -> Result<()> {
        while let Ok(command) = self.rpc.try_recv() {
            self.run(command, channels)?;
        }
        Ok(())
    }

    fn run(&self, command: InternalCommand, channels: &Channels) -> Result<()> {
        use InternalCommand::*;

        trace!("Handling internal RPC command: {:?}", command);
        match command {
            CloseChannel(channel_id, reply_code, reply_text) => channels
                .get(channel_id)
                .map(|channel| {
                    self.handle
                        .register_internal_future(channel.close(reply_code, &reply_text))
                })
                .unwrap_or(Ok(())),
            CloseConnection(reply_code, reply_text, class_id, method_id) => channels
                .get(0)
                .map(|channel0| {
                    self.handle
                        .register_internal_future(channel0.connection_close(
                            reply_code,
                            &reply_text,
                            class_id,
                            method_id,
                        ))
                })
                .unwrap_or(Ok(())),
            SendConnectionCloseOk(error) => channels
                .get(0)
                .map(|channel| {
                    self.handle
                        .register_internal_future(channel.connection_close_ok(error))
                })
                .unwrap_or(Ok(())),
            RemoveChannel(channel_id, error) => channels.remove(channel_id, error),
            SetConnectionClosing => {
                channels.set_connection_closing();
                Ok(())
            }
            SetConnectionClosed(error) => channels.set_connection_closed(error),
            SetConnectionError(error) => channels.set_connection_error(error),
        }
    }
}
