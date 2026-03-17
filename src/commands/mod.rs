pub mod devices;
pub mod play;
pub mod queue;
pub mod search;
pub mod volume;

pub fn join_artist_names(artists: &[rspotify::model::SimplifiedArtist]) -> String {
    artists
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn release_year(date: Option<&str>) -> Option<&str> {
    date.and_then(|d| d.get(..4))
}
