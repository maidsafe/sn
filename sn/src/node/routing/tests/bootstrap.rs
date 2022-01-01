// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

mod utils;

use anyhow::{Error, Result};
use ed25519_dalek::Keypair;
use futures::future;
use crate::node::routing::routing_api::{Config, Event, NodeElderChange};
use std::collections::HashSet;
use tokio::time;
use utils::*;
use xor_name::XOR_NAME_LEN;
use crate::elder_count;

/*
#[tokio::test(flavor = "multi_thread")]
async fn test_genesis_node() -> Result<()> {
    let keypair = Keypair::generate(&mut rand::thread_rng());
    let (node, mut event_stream) = create_node(Config {
        first: true,
        keypair: Some(keypair),
        ..Default::default()
    })
    .await?;

    assert_next_event!(event_stream, Event::EldersChanged { .. });

    assert!(node.is_elder().await);

    assert_eq!(node.name().await[XOR_NAME_LEN - 1], 255);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_node_bootstrapping() -> Result<()> {
    let (genesis_node, mut event_stream) = create_node(Config {
        first: true,
        ..Default::default()
    })
    .await?;

    // spawn genesis node events listener
    let genesis_handler = tokio::spawn(async move {
        assert_next_event!(event_stream, Event::EldersChanged { .. });

        assert_next_event!(event_stream, Event::MemberJoined { .. });
    });

    // bootstrap a second node with genesis
    let genesis_contact = genesis_node.our_connection_info();
    let (node1, _event_stream) = create_node(config_with_contact(genesis_contact)).await?;

    // just await for genesis node to finish receiving all events
    genesis_handler.await?;

    let elder_size = 2;
    verify_invariants_for_node(&genesis_node, elder_size).await?;
    verify_invariants_for_node(&node1, elder_size).await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_startup_section_bootstrapping() -> Result<()> {
    // Create the genesis node.
    let (genesis_node, mut event_stream) = create_node(Config {
        first: true,
        ..Default::default()
    })
    .await?;
    let other_node_count =elder_count() - 1;

    // Then add more nodes to form a section. Because there is only `elder_count()` nodes in total,
    // we expect every one to be promoted to elder.
    let genesis_contact = genesis_node.our_connection_info();
    let nodes_joining_tasks = (0..other_node_count).map(|_| async {
        let (node, mut event_stream) = create_node(config_with_contact(genesis_contact)).await?;
        assert_event!(
            event_stream,
            Event::EldersChanged {
                self_status_change: NodeElderChange::Promoted,
                ..
            }
        );
        Ok::<_, Error>(node)
    });
    let other_nodes = future::try_join_all(nodes_joining_tasks).await?;

    // Keep track of the joined nodes the genesis node knows about.
    let mut joined_names = HashSet::new();

    // Keep listening to the events from the genesis node until it becomes aware of all the other
    // nodes in the section.
    while let Some(event) = time::timeout(TIMEOUT, event_stream.next()).await? {
        let _ = match event {
            Event::MemberJoined { name, .. } => joined_names.insert(name),
            Event::MemberLeft { name, .. } => joined_names.remove(&name),
            _ => false,
        };

        let actual_names: HashSet<_> = future::join_all(other_nodes.iter().map(|node| node.name()))
            .await
            .into_iter()
            .collect();

        if joined_names == actual_names {
            return Ok(());
        }
    }

    panic!("event stream unexpectedly closed")
}

// Test that the first `elder_count()` nodes in the network are promoted to elders.
#[tokio::test(flavor = "multi_thread")]
async fn test_startup_elders() -> Result<()> {
    let mut nodes = create_connected_nodes(elder_count()).await?;

    future::join_all(nodes.iter_mut().map(|(node, stream)| async move {
        if node.is_elder().await {
            return;
        }

        assert_event!(
            stream,
            Event::EldersChanged {
                self_status_change: NodeElderChange::Promoted,
                ..
            }
        )
    }))
    .await;

    Ok(())
}
*/
