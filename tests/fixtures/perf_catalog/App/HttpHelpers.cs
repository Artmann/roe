using System;
using System.Collections.Generic;
using System.Linq;
using System.Net.Http;
using System.Text.RegularExpressions;
using System.Threading;
using System.Threading.Tasks;

namespace App;

/// Every check that fires outside a loop.
public class HttpHelpers
{
    public string Fetch(string url)
    {
        var client = new HttpClient();

        return client.GetStringAsync(url).Result;
    }

    public async Task WarmUpAsync()
    {
        Thread.Sleep(100);

        await Task.CompletedTask;
    }

    public bool Matches(string value, string other)
    {
        var pattern = new Regex("^[a-z]+$");

        return pattern.IsMatch(value) && value.ToLower() == other.ToLower();
    }

    public bool HasAny(IEnumerable<string> values)
    {
        return values.Count() > 0;
    }

    public string FirstNonEmpty(IEnumerable<string> values)
    {
        return values.Where(value => value.Length > 0).First();
    }
}
