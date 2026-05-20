"""sample fixture."""
import os

ANSWER = 42

@no_type_check
def free(x):
    """free fn."""
    return x + 1

class Foo:
    """doc."""
    def double(self, n):
        return n * 2

    @classmethod
    def name(cls):
        return "foo"

class Outer:
    class Inner:
        def helper(self):
            return True

def with_decorator():
    pass
