using Xunit;

namespace Lib.Tests;

public class CalculatorTests
{
    [Fact]
    public void Adds()
    {
        var calculator = new Calculator();
        Assert.Equal(3, calculator.Add(1, 2));
    }
}
