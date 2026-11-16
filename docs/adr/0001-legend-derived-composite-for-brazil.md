---
status: accepted
---

# Legend-derived composite for Brazil

Brazil's only open radar data is INPE/CPTEC's Nowcasting service, which publishes each radar's CAPPI 3 km reflectivity as a precoloured PNG and no measured-value product or national mosaic. Grid sources otherwise exclude precoloured imagery, but we accept this one as a narrow exception: a national product with no measured-value alternative, whose published legend matches an accepted profile exactly for every layer, and whose frames are rejected when unknown colours appear. Omastorm recovers legend-derived reflectivity (exact to the legend's 1 dBZ resolution) and builds its own composite, which follow picks automatically inside Brazil.

## Considered options

- **Leave Brazil uncovered.** Honest, but the only open data would go unused.
- **Manual-only, like MRMS.** More cautious, but nothing else covers Brazil, so follow would show an empty map there anyway.
- **Accept any precoloured source.** Rejected: provider-side smoothing and unknown palettes would turn colour guesses into reflectivity.

## Consequences

The composite carries a nominal time covering the 20 minutes before it, and the source keeps its own age rules (stale at 45 minutes, unavailable at 90) because INPE delivers 10 to 40 minutes after observation. A legend change at INPE drops that radar from the composite (logged, not reinterpreted) until the accepted profile is updated.
