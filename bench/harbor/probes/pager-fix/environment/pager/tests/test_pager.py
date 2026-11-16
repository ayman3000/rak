import pytest

from pager import page_count, paginate

ITEMS = list(range(1, 26))  # 25 items


def test_first_page():
    assert paginate(ITEMS, 1, 10) == list(range(1, 11))


def test_second_page():
    assert paginate(ITEMS, 2, 10) == list(range(11, 21))


def test_last_page_is_partial():
    assert paginate(ITEMS, 3, 10) == [21, 22, 23, 24, 25]


def test_past_the_end_is_empty():
    assert paginate(ITEMS, 4, 10) == []


def test_page_count():
    assert page_count(ITEMS, 10) == 3
    assert page_count([], 10) == 0


def test_bad_page():
    with pytest.raises(ValueError):
        paginate(ITEMS, 0, 10)
