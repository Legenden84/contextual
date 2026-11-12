# ScreenWelcome

The empty state, shown when no project is open: Open folder (native picker), four steps that explain the app in one breath, the Foundry connection, and recent projects with name, path, stack and last opened.

Each recent project has a trailing × icon button with the tooltip “Remove from recent projects”.
It removes only that entry from history without opening the project or deleting its files or saved
chats. Removal matches both path and location, so identical paths in different WSL distros remain
independent. “Clear recent” still removes the entire list. Both actions persist the updated history;
removing the final entry shows the existing empty state.
