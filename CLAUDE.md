- Use assertions to throw on invalid states.
- Don't use unwrap unless you are 100% sure the value is not null or undefined. Use `invariant` or `assert` instead.
- Use conventional commits. If you are making changes to specific commands, use that as the scope. For example, `fix(dead-code): Avoid matching variables with the same name.`.
- Use semver for versioning. For example, `1.0.0` is the first release, `1.0.1` is a patch, `1.1.0` is a minor release, and `2.0.0` is a major release.

## Code Style

- Don't use CONSTANT_CASE. This is not JAVA.
- Use entire words as variable names. This is not Go. For example `request` instead of `req`.
- Use punctuation.
- Use whitespace to break up code to make it easier to read. Put a blank like after const groups and control flows and before return statements.
- Order things in alphabetical order by default. If applicable order by accessiblity level first, then alphabetical order.
- No Floating Promises: Always await or handle promises
- Always use bracers for control statements.

## Error handling

- Always handle errors.
- User facing errors should be easy to understand and actionable.
- Error messages must be **actionable** — tell the user what went wrong and what they can do about it
- When planning features, always consider what errors can occur and include the exact error messages in the plan

## Testing

- Put test files next to the implementation.
- Unit test small, side effect free modules.
- We prefer "integration tests" that only mocks a small set of dependencies.
- Normally, we test the entire endpoint, using a mock database in esix. A good API test should perform a request and then assert that the correct documents have been created in the database.
