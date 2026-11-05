# src/server

Node-side code for Contextual. There is no standalone backend: this code runs inside
the local Vite dev/preview process (e.g. as a Vite plugin exposing `/api/*` middleware)
and handles things the browser can't — reading the project folder, token counting,
calling the Azure AI Foundry deployment, and writing applied files to disk.

Never import from `src/web` here, and never import this folder from `src/web`;
share types through a dedicated `src/shared` module if needed.
