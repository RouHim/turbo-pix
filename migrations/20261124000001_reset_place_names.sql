-- Place names moved from the network resolver (a Nominatim city string in
-- `metadata.location.city`) to the offline dataset, which stores a composed
-- `place` plus its `country_code`. Names written by the old resolver are not
-- the dataset's answer, so every one is dropped and every row carrying
-- coordinates goes back into the resolver queue: the next scan rebuilds the
-- names in-process, without a network call.
UPDATE photos
SET metadata = json_remove(
        metadata,
        '$.location.city',
        '$.location.place',
        '$.location.country_code'
    ),
    geo_location_resolved = 0;
