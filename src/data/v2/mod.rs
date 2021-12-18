// Copyright (C) 2021 The apca Developers
// SPDX-License-Identifier: GPL-3.0-or-later

/// Definitions for retrieval of market data bars.
pub mod bars;
/// Functionality for retrieval of most recent quotes.
pub mod last_quote;

// TODO: Remove this alias with the next compatibility breaking release.
#[deprecated(note = "renamed to 'bars'; use that instead")]
pub use bars as stocks;