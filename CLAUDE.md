# Project Overview : eigen
A static site generator written in Rust that uses jinja2 templates to create optimized static sites that can be served with any file server and adds fast transitions using htmx and partials that are also rendered to files.

## Tech stack
Templating = Minijinja
Frontend = Htmx driven templates
Async Runtime = tokio

## Local development

Nix, direnv and flake to manage local dev environment
just to run often used commands

## Work Structure
Always create a plan,
then review the plan,
then apply the reviews to the plan,
then create an implementation plan,
review the implementation plan
then apply the implentation reviews
AND then actually start writing code.

Always create a git branch for the work.
Create atomic commits for coherent work done.
Branch does not get merged unless the feature has tests that are passing.
Integration tests (if required, not mandatory) should be in rust as well.
Always write docs for any new feature under `docs/<feature_name>.md` which you can later read for your own reference. These docs are for your reference.
If there any updates to a feature, do not merge unless the docs for the feature are also updated.
Finally, run `/simplify` to make the new code reasonable before commiting.
Use `bd` to track tasks.

## Code Style

- Idiotmatic rust code
- Optimized for readability first
- Avoid long format!() chains and other allocations. Be memory efficient.
- Write tests immediately after a feature.
- Do not write "ceremony" tests that actually just test the library being used.
- Do not use unwrap or expect unless its an invariant.
- Read the docs for the libraries to plan the implementation.

## Core concepts

- eigen reads the template files and creates a "sitemap" of the website based on template names and their frontmatter.
- it fetches data (if the frontmatter asks for it) from multiple sources (local or remote) and renders the jinja templates into html.
- if there are {% block %} in the template, eigen also renders them into a separate partial so that each block can be replaced with an htmx get if needed.
- eigen optimizes images, html, js and css.
- the dist folder should be deployable as is by cdns

## Available commands
The just file has available commands. Be mindful of commands that you run often, add it to the justfile. Adjust the justfile to match commands that you use often.
