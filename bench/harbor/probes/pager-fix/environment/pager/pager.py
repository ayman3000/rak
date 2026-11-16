"""Splitting a list into numbered pages."""


def page_count(items, per_page):
    """How many pages `items` fill at `per_page` items a page."""
    if per_page < 1:
        raise ValueError("per_page must be at least 1")
    return (len(items) + per_page - 1) // per_page


def paginate(items, page, per_page):
    """The items on page `page`, counting from 1; an empty list past the end."""
    if page < 1 or per_page < 1:
        raise ValueError("page and per_page must be at least 1")
    start = page * per_page
    return items[start:start + per_page]
