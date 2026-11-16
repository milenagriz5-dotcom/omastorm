# Omastorm

A desktop radar viewer: a local engine fetches public radar data and the bar UI draws it. These terms name what the user sees and what the engine serves.

## Sources and what they cover

**Source**:
One provider feed compiled into the engine, either a network of individual radars or a single georeferenced grid.
_Avoid_: provider (for the feed), layer

**Site**:
One radar dish whose sweeps are shown on their own.
_Avoid_: station (outside NEXRAD's own naming), radar (for the dish as a selection)

**Mosaic**:
A georeferenced grid of reflectivity covering many radars at once, shown as one picture.
_Avoid_: composite (unless Omastorm built it), tile set

**Composite**:
A mosaic that Omastorm assembles itself from separate per-radar products, because the provider publishes no mosaic of its own.
_Avoid_: mosaic (when the distinction matters), merge

## Time

**Observation time**:
When the radar measured what a frame shows.
_Avoid_: scan time (in prose), timestamp

**Nominal time**:
The time a mosaic or composite is stamped with, standing for the observations of a fixed window before it rather than one instant.
_Avoid_: observation time (for mosaics), slot

## Values

**Legend-derived reflectivity**:
Reflectivity recovered from a provider's coloured image through its published legend, exact to that legend's resolution and labelled as such.
_Avoid_: approximate dBZ, estimated reflectivity
