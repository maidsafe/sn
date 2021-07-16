// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use super::{
    chunk_records::ChunkRecords, map_storage::MapStorage, register_storage::RegisterStorage,
    sequence_storage::SequenceStorage,
};
use crate::messaging::{
    data::{DataCmd, DataExchange, DataQuery, MapCmd, RegisterCmd, SequenceCmd},
    ClientSigned, EndUser, MessageId,
};
use crate::node::{node_ops::NodeDuty, Error, Result};
use crate::routing::Prefix;
use crate::types::PublicKey;
use tracing::info;

/// The various data type stores,
/// that are only managed at Elders.
pub(super) struct ElderStores {
    chunk_records: ChunkRecords,
    map_storage: MapStorage,
    sequence_storage: SequenceStorage,
    register_storage: RegisterStorage,
}

impl ElderStores {
    pub(super) fn new(
        chunk_records: ChunkRecords,
        map_storage: MapStorage,
        sequence_storage: SequenceStorage,
        register_storage: RegisterStorage,
    ) -> Self {
        Self {
            chunk_records,
            map_storage,
            sequence_storage,
            register_storage,
        }
    }

    pub(super) async fn read(
        &self,
        query: DataQuery,
        msg_id: MessageId,
        requester: PublicKey,
        origin: EndUser,
    ) -> Result<NodeDuty> {
        match &query {
            DataQuery::Blob(read) => self.chunk_records.read(read, msg_id, origin).await,
            DataQuery::Map(read) => self.map_storage.read(read, msg_id, requester, origin).await,
            DataQuery::Sequence(read) => {
                self.sequence_storage
                    .read(read, msg_id, requester, origin)
                    .await
            }
            DataQuery::Register(read) => {
                self.register_storage
                    .read(read, msg_id, requester, origin)
                    .await
            }
        }
    }

    pub(super) async fn write(
        &mut self,
        cmd: DataCmd,
        msg_id: MessageId,
        client_sig: ClientSigned,
        origin: EndUser,
    ) -> Result<NodeDuty> {
        info!("Writing Data");
        match cmd {
            DataCmd::Chunk(write) => {
                info!("Writing Blob");
                self.chunk_records
                    .write(write, msg_id, client_sig, origin)
                    .await
            }
            DataCmd::Map(write) => {
                info!("Writing Map");
                self.map_storage
                    .write(msg_id, origin, MapCmd { write, client_sig })
                    .await
            }
            DataCmd::Sequence(write) => {
                info!("Writing Sequence");
                self.sequence_storage
                    .write(msg_id, origin, SequenceCmd { write, client_sig })
                    .await
            }
            DataCmd::Register(write) => {
                info!("Writing Register");
                self.register_storage
                    .write(msg_id, origin, RegisterCmd { write, client_sig })
                    .await
            }
        }
    }

    pub(super) fn chunk_records(&self) -> &ChunkRecords {
        &self.chunk_records
    }

    // NB: Not yet including Register metadata.
    pub(super) async fn get_data_of(&self, prefix: Prefix) -> Result<DataExchange> {
        // Prepare chunk_records, map and sequence data
        let chunk_data = self.chunk_records.get_data_of(prefix).await;
        let map_data = self.map_storage.get_data_of(prefix).await?;
        let reg_data = self.register_storage.get_data_of(prefix).await?;
        let seq_data = self.sequence_storage.get_data_of(prefix).await?;

        Ok(DataExchange {
            chunk_data,
            map_data,
            reg_data,
            seq_data,
        })
    }

    pub(super) async fn update(&mut self, data: DataExchange) -> Result<(), Error> {
        // todo: all this can be done in parallel
        self.map_storage.update(data.map_data).await?;
        self.register_storage.update(data.reg_data).await?;
        self.sequence_storage.update(data.seq_data).await?;
        self.chunk_records.update(data.chunk_data).await;
        Ok(())
    }
}
