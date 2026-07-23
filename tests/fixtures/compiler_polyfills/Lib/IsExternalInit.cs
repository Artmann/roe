namespace System.Runtime.CompilerServices;

// Polyfill required to use `init` accessors and `record` types on
// netstandard2.1, where this type doesn't ship in the BCL. Only the
// compiler ever references it (emitted into IL for `init` members), so it
// is never named anywhere in source.
internal static class IsExternalInit
{
}
