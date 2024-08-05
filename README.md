# BPCon Rust Library

[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](https://www.gnu.org/licenses/gpl-3.0)

This is a generic rust implementation of the `BPCon` consensus mechanism.

## Library Structure

### src/party.rs

Main entity in this implementation is `Party` - it represents member of the consensus.

External system shall create desired amount of parties. 

We have 2 communication channels - one for sending `MessageWire` - encoded in bytes message and routing information, 
and the other for pitching consensus events - this allows for external system to impose custom limitations and rules
regarding runway.

### src/message.rs

Definitions of the general message struct, routing information and type-specific contents.  

### src/lib.rs

Here we present a trait for the value on which consensus is being conducted. Additionally, there is a trait for
defining custom value selection rules, called `ValueSelector`.

