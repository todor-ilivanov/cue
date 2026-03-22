Implementation Plan: Album and Playlist Search in the Player TUI

       Overview

       Add album and playlist search to the in-player TUI search (the / mode in player.rs). The user
       cycles search type with Tab while in SearchInput mode. The search bar visually indicates which type
        is active. Selected albums/playlists start context playback rather than URI playback.

       ---
       1. Data Structure Changes

       Replace SearchResultEntry with an enum-based model.

       The current SearchResultEntry holds a track_id: TrackId<'static>. This needs to generalize to hold
       track, album, or playlist identifiers. The cleanest approach is an enum for the playback target,
       keeping the display fields (title, subtitle) uniform:

       enum SearchPlayTarget {
           Track(TrackId<'static>),
           Album(AlbumId<'static>),
           Playlist(PlaylistId<'static>),
       }

       struct SearchResultEntry {
           title: String,
           subtitle: String,   // replaces "artist" -- used for artist on tracks/albums, owner on
       playlists
           target: SearchPlayTarget,
       }

       Rationale for a flat struct with an enum target rather than three separate Vec types: the rendering
        and navigation code stays uniform. The selected index works identically. The only branching point
       is when playing the result, which the deferred action already handles cleanly.

       Add a SearchCategory enum to track what the user is searching for:

       #[derive(Clone, Copy, PartialEq, Eq)]
       enum SearchCategory {
           Track,
           Album,
           Playlist,
       }

       impl SearchCategory {
           fn next(self) -> Self {
               match self {
                   Self::Track => Self::Album,
                   Self::Album => Self::Playlist,
                   Self::Playlist => Self::Track,
               }
           }

           fn label(self) -> &'static str {
               match self {
                   Self::Track => "Track",
                   Self::Album => "Album",
                   Self::Playlist => "Playlist",
               }
           }
       }

       ---
       2. PlayerMode Changes

       Add category: SearchCategory to each search-related variant:

       enum PlayerMode {
           Normal,
           SearchInput {
               query: String,
               category: SearchCategory,
           },
           SearchLoading {
               query: String,
               category: SearchCategory,
           },
           SearchResults {
               query: String,
               category: SearchCategory,
               results: Vec<SearchResultEntry>,
               selected: usize,
           },
       }

       The category flows through all three search states so (a) the loading overlay can show what type is
        being searched, and (b) re-searching (if you wanted to add that later) preserves context.

       Initialization change (line 656): When pressing /, initialize with SearchCategory::Track as
       default:
       mode = PlayerMode::SearchInput {
           query: String::new(),
           category: SearchCategory::Track,
       };

       ---
       3. perform_search Modifications

       Replace the single perform_search function with one that takes a SearchCategory parameter. This
       function dispatches to the appropriate Spotify API call:

       fn perform_search(
           spotify: &AuthCodeSpotify,
           query: &str,
           category: SearchCategory,
       ) -> Result<Vec<SearchResultEntry>, String> {
           match category {
               SearchCategory::Track => perform_track_search(spotify, query),
               SearchCategory::Album => perform_album_search(spotify, query),
               SearchCategory::Playlist => perform_playlist_search(spotify, query),
           }
       }

       perform_track_search -- identical to the current perform_search body (lines 302-333), but
       constructs SearchResultEntry with target: SearchPlayTarget::Track(id) and subtitle instead of
       artist.

       perform_album_search -- mirrors play_album in play.rs (lines 82-134):
       fn perform_album_search(
           spotify: &AuthCodeSpotify,
           query: &str,
       ) -> Result<Vec<SearchResultEntry>, String> {
           let result = spotify
               .search(query, SearchType::Album, None, None, Some(5), None)
               .map_err(|e| format!("search failed: {e}"))?;

           let albums = match result {
               SearchResult::Albums(page) => page,
               _ => return Err("unexpected search result type".to_string()),
           };

           let entries: Vec<SearchResultEntry> = albums
               .items
               .into_iter()
               .filter_map(|a| {
                   let id = a.id?;
                   Some(SearchResultEntry {
                       title: a.name,
                       subtitle: join_artist_names(&a.artists),
                       target: SearchPlayTarget::Album(id),
                   })
               })
               .collect();

           if entries.is_empty() {
               Err(format!("no albums for \"{query}\""))
           } else {
               Ok(entries)
           }
       }

       perform_playlist_search -- must use crate::client::search_playlists() (the null-filtering wrapper),
        matching the pattern from play.rs lines 136-172:
       fn perform_playlist_search(
           spotify: &AuthCodeSpotify,
           query: &str,
       ) -> Result<Vec<SearchResultEntry>, String> {
           let playlists = crate::client::search_playlists(spotify, query, 5)
               .map_err(|e| format!("search failed: {e}"))?;

           let entries: Vec<SearchResultEntry> = playlists
               .items
               .into_iter()
               .map(|p| SearchResultEntry {
                   title: p.name,
                   subtitle: p.owner.display_name.unwrap_or_else(|| "unknown".to_string()),
                   target: SearchPlayTarget::Playlist(p.id),
               })
               .collect();

           if entries.is_empty() {
               Err(format!("no playlists for \"{query}\""))
           } else {
               Ok(entries)
           }
       }

       Note: Playlist IDs are not optional in rspotify's SimplifiedPlaylist (unlike album/track IDs), so
       no filter_map is needed there.

       ---
       4. Input Handling Changes

       4a. Tab key in SearchInput mode (around line 793-812).

       Add a KeyCode::Tab arm inside the PlayerMode::SearchInput { query, category } match:

       KeyCode::Tab | KeyCode::BackTab => {
           *category = category.next();
           needs_redraw = true;
       }

       This cycles Track -> Album -> Playlist -> Track. BackTab (Shift+Tab) could cycle in reverse if
       desired, but starting with forward-only cycling via next() is sufficient and simpler.

       4b. Search submission (around line 802-806).

       The deferred submit_search must also carry the category. Change the type from Option<String> to
       Option<(String, SearchCategory)>:

       let mut submit_search: Option<(String, SearchCategory)> = None;

       In the SearchInput Enter handler:
       KeyCode::Enter => {
           if !query.is_empty() {
               submit_search = Some((query.clone(), *category));
           }
       }

       And the deferred search submission block (lines 849-860):
       if let Some((q, cat)) = submit_search {
           let sp = spotify.clone();
           let query = q.clone();
           let (tx, rx) = mpsc::channel();
           search_rx = Some(rx);
           std::thread::spawn(move || {
               let result = perform_search(&sp, &query, cat);
               let _ = tx.send(result);
           });
           mode = PlayerMode::SearchLoading { query: q, category: cat };
           needs_redraw = true;
       }

       4c. Deferred play action (lines 862-878).

       Replace play_track_id: Option<TrackId<'static>> with play_target: Option<SearchPlayTarget>:

       let mut play_target: Option<SearchPlayTarget> = None;

       In the SearchResults Enter/number-key handlers:
       KeyCode::Enter => {
           play_target = Some(results[*selected].target.clone());
       }
       KeyCode::Char(c @ '1'..='9') => {
           let idx = (c as usize) - ('1' as usize);
           if idx < results.len() {
               play_target = Some(results[idx].target.clone());
           }
       }

       The deferred play block becomes:
       if let Some(target) = play_target {
           let result = match target {
               SearchPlayTarget::Track(id) => {
                   let playable = PlayableId::Track(id);
                   spotify.start_uris_playback([playable], None, None, None)
               }
               SearchPlayTarget::Album(id) => {
                   let context = PlayContextId::Album(id);
                   spotify.start_context_playback(context, None, None, None)
               }
               SearchPlayTarget::Playlist(id) => {
                   let context = PlayContextId::Playlist(id);
                   spotify.start_context_playback(context, None, None, None)
               }
           };
           match result {
               Err(e) => {
                   status_message = Some((
                       format!("{}", api_error(e, "start playback")),
                       Instant::now(),
                   ));
               }
               Ok(()) => {
                   deferred_fetch = Some(Instant::now() + Duration::from_millis(800));
               }
           }
           mode = PlayerMode::Normal;
           needs_redraw = true;
       }

       4d. SearchLoading category passthrough (lines 971-977).

       When search results arrive, the mode transition from SearchLoading to SearchResults must carry
       category:
       if let PlayerMode::SearchLoading { query, category } = &mode {
           mode = PlayerMode::SearchResults {
               query: query.clone(),
               category: *category,
               results,
               selected: 0,
           };
       }

       Same for the error case (lines 983-987):
       if let PlayerMode::SearchLoading { query, category } = &mode {
           mode = PlayerMode::SearchInput {
               query: query.clone(),
               category: *category,
           };
       }

       ---
       5. Rendering Changes

       5a. draw_search_input_bar (lines 335-367).

       Add a category: SearchCategory parameter. Show the category tabs in the search bar. The three
       labels ("Track", "Album", "Playlist") are rendered inline, with the active one in accent color and
       the others dim:

       fn draw_search_input_bar(frame: &mut Frame, query: &str, category: SearchCategory) {
           // ... same area calculation ...

           let key_style = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);
           let desc_style = Style::new().fg(Color::DarkGray);
           let dim_style = Style::new().fg(Color::DarkGray).add_modifier(Modifier::DIM);
           let active_style = Style::new().fg(ACCENT);

           // Category indicator spans
           let categories = [SearchCategory::Track, SearchCategory::Album, SearchCategory::Playlist];
           let mut cat_spans: Vec<Span> = Vec::new();
           for (i, &cat) in categories.iter().enumerate() {
               if i > 0 {
                   cat_spans.push(Span::styled("/", dim_style));
               }
               if cat == category {
                   cat_spans.push(Span::styled(cat.label(), active_style));
               } else {
                   cat_spans.push(Span::styled(cat.label(), dim_style));
               }
           }
           cat_spans.push(Span::styled(" ", Style::new()));

           let mut left = cat_spans;
           left.push(Span::styled("/ ", key_style));
           left.push(Span::styled(query.to_string(), Style::new().fg(Color::White)));
           left.push(Span::styled("_", Style::new().fg(Color::DarkGray)));

           // Right side hints add Tab
           let right_parts = vec![
               Span::styled("Tab", key_style),
               Span::styled(" type  ", desc_style),
               Span::styled("Enter", key_style),
               Span::styled(" search  ", desc_style),
               Span::styled("Esc", key_style),
               Span::styled(" cancel", desc_style),
           ];

           // ... padding and render same as before ...
       }

       The visual result in the search bar: Track/Album/Playlist / my query_                Tab type
       Enter search  Esc cancel with the active type highlighted in amber.

       5b. draw_search_results_overlay (lines 384-425).

       Change the signature to accept &[SearchResultEntry] (same) and category: SearchCategory. The
       rendering adapts the subtitle label based on type:

       - Track: #. Title -- Artist (same as now)
       - Album: #. Title -- Artist (same format, subtitle is artist)
       - Playlist: #. Title -- by Owner

       Since the subtitle field already holds the right content (artist for tracks/albums, owner for
       playlists), the only change is a formatting tweak for playlists. One clean way: prefix "by " for
       playlists in the subtitle at construction time in perform_playlist_search, so the renderer stays
       uniform. Then the overlay code changes only in its parameter list (add category), and the subtitle
       rendering stays as-is: entry.title -- entry.subtitle.

       Actually, looking at this again: the existing renderer shows entry.title and entry.artist. Simply
       rename artist to subtitle in the entry access and use entry.subtitle. The content is correct
       because we set it to "by {owner}" for playlists in the search function. This is the simplest path.

       5c. Call site updates in the draw block (lines 1054-1067).

       Update calls to pass category:
       PlayerMode::SearchInput { query, category } => {
           draw_search_input_bar(frame, query, *category);
       }
       PlayerMode::SearchResults { results, selected, category, .. } => {
           draw_search_results_overlay(frame, results, *selected, *category);
       }

       ---
       6. Import Changes

       Add to the existing imports at the top of player.rs (line 8):

       use rspotify::model::{AlbumId, PlayContextId, PlayableId, PlayableItem, PlaylistId, SearchResult,
       SearchType, TrackId};

       The current line imports PlayableId, PlayableItem, SearchResult, SearchType, TrackId. Add AlbumId,
       PlayContextId, and PlaylistId.

       ---
       7. SearchPlayTarget Derive Considerations

       SearchPlayTarget needs Clone so it can be moved out of the results vec into the deferred action.
       The rspotify ID types (TrackId, AlbumId, PlaylistId) all implement Clone, so this works:

       #[derive(Clone)]
       enum SearchPlayTarget {
           Track(TrackId<'static>),
           Album(AlbumId<'static>),
           Playlist(PlaylistId<'static>),
       }

       ---
       8. Test Updates

       The existing tests in the tests module (lines 1084+) test current_progress_ms and
       progress_bar_width, neither of which is affected by these changes. No test updates are required for
        the existing tests.

       New tests to consider adding:
       - SearchCategory::next() cycles correctly through all three variants.
       - Verify SearchCategory::next() wraps from Playlist back to Track.

       These are trivial unit tests and can be added to the #[cfg(test)] mod tests block.

       ---
       9. Sequencing and Dependencies

       The implementation order should be:

       1. Add SearchCategory enum and SearchPlayTarget enum and SearchResultEntry struct changes (lines
       24-44 area).
       2. Update PlayerMode variants to include category.
       3. Update perform_search to accept SearchCategory, add perform_album_search and
       perform_playlist_search helper functions.
       4. Update imports to include AlbumId, PlayContextId, PlaylistId.
       5. Update input handling in run_player_loop: Tab in SearchInput, deferred action types, search
       submission, result arrival.
       6. Update rendering functions: draw_search_input_bar, draw_search_results_overlay, and their call
       sites.
       7. Verify with cargo clippy and cargo fmt --check.

       Steps 1-4 are pure additions. Step 5 is the bulk of the logic change. Step 6 is visual adaptation.

       ---
       10. Potential Challenges

       Thread safety of search_playlists: The playlist search function (crate::client::search_playlists)
       clones the access token from the spotify client internally and makes its own HTTP request via ureq.
        Since perform_search already runs in a spawned thread with a cloned AuthCodeSpotify, this works
       identically to how the track search works. No issue here.

       Search bar width: Adding the category indicator ("Track/Album/Playlist ") adds about 22 characters
       to the left side of the search bar. On very narrow terminals (<50 columns), this could crowd the
       right-side hints. The existing right-side hint width calculation will handle this via the padding
       math -- padding would go to zero and hints would overlap, but the same issue already exists for
       long queries. The compact hint path (width < 85) already exists in build_hints_playing for the
       normal mode hints, and a similar approach could be taken here. For a first pass, adding "Tab type"
       to the right hints and the category labels on the left is fine; narrow terminal polish can come
       later.

       search_rx type change: The search_rx channel type at line 628 is
       Option<mpsc::Receiver<Result<Vec<SearchResultEntry>, String>>>. The SearchResultEntry type changes
       but the channel type signature stays the same since the struct name is unchanged. No issue.
