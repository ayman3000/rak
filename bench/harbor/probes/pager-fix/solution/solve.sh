#!/bin/bash
sed -i "s/start = page \* per_page/start = (page - 1) * per_page/" /app/pager/pager.py
