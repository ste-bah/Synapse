# Retired Synapse Pico HID Firmware

The physical Pico HID path is retired. Current Synapse action uses the
software backend (`SendInput`, UIA, and browser/CDP paths); the legacy
`hardware` backend token remains only as a fail-closed compatibility value that
returns `ACTION_BACKEND_UNAVAILABLE`.

This directory intentionally contains no buildable firmware workspace. The old
`firmware/pico-hid/` sources, release helper scripts, and UF2 artifact path are
not part of current `main`. The historical M4 firmware notes remain in
`CHANGELOG.md` for provenance, but they are not setup instructions for this
checkout.
