using System;
using System.Collections.Generic;
using System.Text.RegularExpressions;

namespace Advice;

/// Nothing here is certain enough to fail a build: a regex per call, a local
/// clock read, and a scan of a list.
public class Reporter
{
    private readonly List<string> _codes = new List<string>();

    public string Describe(string[] inputs)
    {
        var pattern = new Regex("^[0-9]+$");
        var stamp = DateTime.Now;

        foreach (var input in inputs)
        {
            if (_codes.Contains(input) && pattern.IsMatch(input))
            {
                return input;
            }
        }

        return stamp.ToString();
    }
}
